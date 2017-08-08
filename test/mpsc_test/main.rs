#![feature(optin_builtin_traits)]
extern crate xpsupport_sys;

use std::sync::Arc;
use std::error;
use std::fmt;
use std::mem;
use std::cell::UnsafeCell;
use std::time::{Duration, Instant};
use std::marker::PhantomData;


// extern crate parking_lot;

pub use self::select::{Select, Handle};
use self::select::StartResult;
use self::select::StartResult::*;
use self::blocking::SignalToken;

mod blocking;
mod oneshot;
mod select;
mod shared;
mod stream;
mod sync;
mod mpsc_queue;
mod spsc_queue;

pub struct Receiver<T> {
    inner: UnsafeCell<Flavor<T>>,
    not_send_sync: PhantomData<*const ()>,
}

// The receiver port can be sent from place to place, so long as it
// is not used to receive non-sendable things.
unsafe impl<T: Send> Send for Receiver<T> { }


#[derive(Debug)]
pub struct Iter<'a, T: 'a> {
    rx: &'a Receiver<T>
}

#[derive(Debug)]
pub struct TryIter<'a, T: 'a> {
    rx: &'a Receiver<T>
}

#[derive(Debug)]
pub struct IntoIter<T> {
    rx: Receiver<T>
}

pub struct Sender<T> {
    inner: UnsafeCell<Flavor<T>>,
    not_send_sync: PhantomData<*const ()>,
}

// The send port can be sent from place to place, so long as it
// is not used to send non-sendable things.
unsafe impl<T: Send> Send for Sender<T> { }

//impl<T> !Sync for Sender<T> { }

pub struct SyncSender<T> {
    inner: Arc<sync::Packet<T>>,
}

unsafe impl<T: Send> Send for SyncSender<T> {}

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct SendError<T>(pub T);


#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub struct RecvError;


#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum TryRecvError {
    /// This **channel** is currently empty, but the **Sender**(s) have not yet
    /// disconnected, so data may yet become available.
    Empty,

    /// The **channel**'s sending half has become disconnected, and there will
    /// never be any more data received on it.
    Disconnected,
}


#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum RecvTimeoutError {
    /// This **channel** is currently empty, but the **Sender**(s) have not yet
    /// disconnected, so data may yet become available.
    Timeout,
    /// The **channel**'s sending half has become disconnected, and there will
    /// never be any more data received on it.
    Disconnected,
}


#[derive(PartialEq, Eq, Clone, Copy)]
pub enum TrySendError<T> {
    Full(T),
    Disconnected(T),
}

enum Flavor<T> {
    Oneshot(Arc<oneshot::Packet<T>>),
    Stream(Arc<stream::Packet<T>>),
    Shared(Arc<shared::Packet<T>>),
    Sync(Arc<sync::Packet<T>>),
}

#[doc(hidden)]
trait UnsafeFlavor<T> {
    fn inner_unsafe(&self) -> &UnsafeCell<Flavor<T>>;
    unsafe fn inner_mut(&self) -> &mut Flavor<T> {
        &mut *self.inner_unsafe().get()
    }
    unsafe fn inner(&self) -> &Flavor<T> {
        &*self.inner_unsafe().get()
    }
}
impl<T> UnsafeFlavor<T> for Sender<T> {
    fn inner_unsafe(&self) -> &UnsafeCell<Flavor<T>> {
        &self.inner
    }
}
impl<T> UnsafeFlavor<T> for Receiver<T> {
    fn inner_unsafe(&self) -> &UnsafeCell<Flavor<T>> {
        &self.inner
    }
}


/// ```
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let a = Arc::new(oneshot::Packet::new());
    (Sender::new(Flavor::Oneshot(a.clone())), Receiver::new(Flavor::Oneshot(a)))
}


pub fn sync_channel<T>(bound: usize) -> (SyncSender<T>, Receiver<T>) {
    let a = Arc::new(sync::Packet::new(bound));
    (SyncSender::new(a.clone()), Receiver::new(Flavor::Sync(a)))
}

////////////////////////////////////////////////////////////////////////////////
// Sender
////////////////////////////////////////////////////////////////////////////////

impl<T> Sender<T> {
    fn new(inner: Flavor<T>) -> Sender<T> {
        Sender {
            inner: UnsafeCell::new(inner),
            not_send_sync: PhantomData,
        }
    }

    pub fn send(&self, t: T) -> Result<(), SendError<T>> {
        let (new_inner, ret) = match *unsafe { self.inner() } {
            Flavor::Oneshot(ref p) => {
                if !p.sent() {
                    return p.send(t).map_err(SendError);
                } else {
                    let a = Arc::new(stream::Packet::new());
                    let rx = Receiver::new(Flavor::Stream(a.clone()));
                    match p.upgrade(rx) {
                        oneshot::UpSuccess => {
                            let ret = a.send(t);
                            (a, ret)
                        }
                        oneshot::UpDisconnected => (a, Err(t)),
                        oneshot::UpWoke(token) => {
                            // This send cannot panic because the thread is
                            // asleep (we're looking at it), so the receiver
                            // can't go away.
                            a.send(t).ok().unwrap();
                            token.signal();
                            (a, Ok(()))
                        }
                    }
                }
            }
            Flavor::Stream(ref p) => return p.send(t).map_err(SendError),
            Flavor::Shared(ref p) => return p.send(t).map_err(SendError),
            Flavor::Sync(..) => unreachable!(),
        };

        unsafe {
            let tmp = Sender::new(Flavor::Stream(new_inner));
            mem::swap(self.inner_mut(), tmp.inner_mut());
        }
        ret.map_err(SendError)
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Sender<T> {
        let packet = match *unsafe { self.inner() } {
            Flavor::Oneshot(ref p) => {
                let a = Arc::new(shared::Packet::new());
                {
                    let guard = a.postinit_lock();
                    let rx = Receiver::new(Flavor::Shared(a.clone()));
                    let sleeper = match p.upgrade(rx) {
                        oneshot::UpSuccess |
                        oneshot::UpDisconnected => None,
                        oneshot::UpWoke(task) => Some(task),
                    };
                    a.inherit_blocker(sleeper, guard);
                }
                a
            }
            Flavor::Stream(ref p) => {
                let a = Arc::new(shared::Packet::new());
                {
                    let guard = a.postinit_lock();
                    let rx = Receiver::new(Flavor::Shared(a.clone()));
                    let sleeper = match p.upgrade(rx) {
                        stream::UpSuccess |
                        stream::UpDisconnected => None,
                        stream::UpWoke(task) => Some(task),
                    };
                    a.inherit_blocker(sleeper, guard);
                }
                a
            }
            Flavor::Shared(ref p) => {
                p.clone_chan();
                return Sender::new(Flavor::Shared(p.clone()));
            }
            Flavor::Sync(..) => unreachable!(),
        };

        unsafe {
            let tmp = Sender::new(Flavor::Shared(packet.clone()));
            mem::swap(self.inner_mut(), tmp.inner_mut());
        }
        Sender::new(Flavor::Shared(packet))
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        match *unsafe { self.inner() } {
            Flavor::Oneshot(ref p) => p.drop_chan(),
            Flavor::Stream(ref p) => p.drop_chan(),
            Flavor::Shared(ref p) => p.drop_chan(),
            Flavor::Sync(..) => unreachable!(),
        }
    }
}

impl<T> fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Sender {{ .. }}")
    }
}

////////////////////////////////////////////////////////////////////////////////
// SyncSender
////////////////////////////////////////////////////////////////////////////////

impl<T> SyncSender<T> {
    fn new(inner: Arc<sync::Packet<T>>) -> SyncSender<T> {
        SyncSender { inner: inner }
    }

    /// ```
    pub fn send(&self, t: T) -> Result<(), SendError<T>> {
        self.inner.send(t).map_err(SendError)
    }

   
    pub fn try_send(&self, t: T) -> Result<(), TrySendError<T>> {
        self.inner.try_send(t)
    }
}

impl<T> Clone for SyncSender<T> {
    fn clone(&self) -> SyncSender<T> {
        self.inner.clone_chan();
        SyncSender::new(self.inner.clone())
    }
}

impl<T> Drop for SyncSender<T> {
fn drop(&mut self) {
    self.inner.drop_chan();
}
}

impl<T> fmt::Debug for SyncSender<T> {
fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "SyncSender {{ .. }}")
}
}

////////////////////////////////////////////////////////////////////////////////
// Receiver
////////////////////////////////////////////////////////////////////////////////

impl<T> Receiver<T> {
    fn new(inner: Flavor<T>) -> Receiver<T> {
        Receiver {
            inner: UnsafeCell::new(inner),
            not_send_sync: PhantomData,
        }
    }

  
    pub fn try_recv(&self) -> Result<T, TryRecvError> {
        loop {
            let new_port = match *unsafe { self.inner() } {
                Flavor::Oneshot(ref p) => {
                    match p.try_recv() {
                        Ok(t) => return Ok(t),
                        Err(oneshot::Empty) => return Err(TryRecvError::Empty),
                        Err(oneshot::Disconnected) => {
                            return Err(TryRecvError::Disconnected)
                        }
                        Err(oneshot::Upgraded(rx)) => rx,
                    }
                }
                Flavor::Stream(ref p) => {
                    match p.try_recv() {
                        Ok(t) => return Ok(t),
                        Err(stream::Empty) => return Err(TryRecvError::Empty),
                        Err(stream::Disconnected) => {
                            return Err(TryRecvError::Disconnected)
                        }
                        Err(stream::Upgraded(rx)) => rx,
                    }
                }
                Flavor::Shared(ref p) => {
                    match p.try_recv() {
                        Ok(t) => return Ok(t),
                        Err(shared::Empty) => return Err(TryRecvError::Empty),
                        Err(shared::Disconnected) => {
                            return Err(TryRecvError::Disconnected)
                        }
                    }
                }
                Flavor::Sync(ref p) => {
                    match p.try_recv() {
                        Ok(t) => return Ok(t),
                        Err(sync::Empty) => return Err(TryRecvError::Empty),
                        Err(sync::Disconnected) => {
                            return Err(TryRecvError::Disconnected)
                        }
                    }
                }
            };
            unsafe {
                mem::swap(self.inner_mut(),
                          new_port.inner_mut());
            }
        }
    }

 
  
    pub fn recv(&self) -> Result<T, RecvError> {
        loop {
            let new_port = match *unsafe { self.inner() } {
                Flavor::Oneshot(ref p) => {
                    match p.recv(None) {
                        Ok(t) => return Ok(t),
                        Err(oneshot::Disconnected) => return Err(RecvError),
                        Err(oneshot::Upgraded(rx)) => rx,
                        Err(oneshot::Empty) => unreachable!(),
                    }
                }
                Flavor::Stream(ref p) => {
                    match p.recv(None) {
                        Ok(t) => return Ok(t),
                        Err(stream::Disconnected) => return Err(RecvError),
                        Err(stream::Upgraded(rx)) => rx,
                        Err(stream::Empty) => unreachable!(),
                    }
                }
                Flavor::Shared(ref p) => {
                    match p.recv(None) {
                        Ok(t) => return Ok(t),
                        Err(shared::Disconnected) => return Err(RecvError),
                        Err(shared::Empty) => unreachable!(),
                    }
                }
                Flavor::Sync(ref p) => return p.recv(None).map_err(|_| RecvError),
            };
            unsafe {
                mem::swap(self.inner_mut(), new_port.inner_mut());
            }
        }
    }

  
    pub fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        // Do an optimistic try_recv to avoid the performance impact of
        // Instant::now() in the full-channel case.
        match self.try_recv() {
            Ok(result)
                => Ok(result),
            Err(TryRecvError::Disconnected)
                => Err(RecvTimeoutError::Disconnected),
            Err(TryRecvError::Empty)
                => self.recv_max_until(Instant::now() + timeout)
        }
    }

    fn recv_max_until(&self, deadline: Instant) -> Result<T, RecvTimeoutError> {
        use self::RecvTimeoutError::*;

        loop {
            let port_or_empty = match *unsafe { self.inner() } {
                Flavor::Oneshot(ref p) => {
                    match p.recv(Some(deadline)) {
                        Ok(t) => return Ok(t),
                        Err(oneshot::Disconnected) => return Err(Disconnected),
                        Err(oneshot::Upgraded(rx)) => Some(rx),
                        Err(oneshot::Empty) => None,
                    }
                }
                Flavor::Stream(ref p) => {
                    match p.recv(Some(deadline)) {
                        Ok(t) => return Ok(t),
                        Err(stream::Disconnected) => return Err(Disconnected),
                        Err(stream::Upgraded(rx)) => Some(rx),
                        Err(stream::Empty) => None,
                    }
                }
                Flavor::Shared(ref p) => {
                    match p.recv(Some(deadline)) {
                        Ok(t) => return Ok(t),
                        Err(shared::Disconnected) => return Err(Disconnected),
                        Err(shared::Empty) => None,
                    }
                }
                Flavor::Sync(ref p) => {
                    match p.recv(Some(deadline)) {
                        Ok(t) => return Ok(t),
                        Err(sync::Disconnected) => return Err(Disconnected),
                        Err(sync::Empty) => None,
                    }
                }
            };

            if let Some(new_port) = port_or_empty {
                unsafe {
                    mem::swap(self.inner_mut(), new_port.inner_mut());
                }
            }

            // If we're already passed the deadline, and we're here without
            // data, return a timeout, else try again.
            if Instant::now() >= deadline {
                return Err(Timeout);
            }
        }
    }

  
    pub fn iter(&self) -> Iter<T> {
        Iter { rx: self }
    }

  
    pub fn try_iter(&self) -> TryIter<T> {
        TryIter { rx: self }
    }

}

impl<T> select::Packet for Receiver<T> {
    fn can_recv(&self) -> bool {
        loop {
            let new_port = match *unsafe { self.inner() } {
                Flavor::Oneshot(ref p) => {
                    match p.can_recv() {
                        Ok(ret) => return ret,
                        Err(upgrade) => upgrade,
                    }
                }
                Flavor::Stream(ref p) => {
                    match p.can_recv() {
                        Ok(ret) => return ret,
                        Err(upgrade) => upgrade,
                    }
                }
                Flavor::Shared(ref p) => return p.can_recv(),
                Flavor::Sync(ref p) => return p.can_recv(),
            };
            unsafe {
                mem::swap(self.inner_mut(),
                          new_port.inner_mut());
            }
        }
    }

    fn start_selection(&self, mut token: SignalToken) -> StartResult {
        loop {
            let (t, new_port) = match *unsafe { self.inner() } {
                Flavor::Oneshot(ref p) => {
                    match p.start_selection(token) {
                        oneshot::SelSuccess => return Installed,
                        oneshot::SelCanceled => return Abort,
                        oneshot::SelUpgraded(t, rx) => (t, rx),
                    }
                }
                Flavor::Stream(ref p) => {
                    match p.start_selection(token) {
                        stream::SelSuccess => return Installed,
                        stream::SelCanceled => return Abort,
                        stream::SelUpgraded(t, rx) => (t, rx),
                    }
                }
                Flavor::Shared(ref p) => return p.start_selection(token),
                Flavor::Sync(ref p) => return p.start_selection(token),
            };
            token = t;
            unsafe {
                mem::swap(self.inner_mut(), new_port.inner_mut());
            }
        }
    }

    fn abort_selection(&self) -> bool {
        let mut was_upgrade = false;
        loop {
            let result = match *unsafe { self.inner() } {
                Flavor::Oneshot(ref p) => p.abort_selection(),
                Flavor::Stream(ref p) => p.abort_selection(was_upgrade),
                Flavor::Shared(ref p) => return p.abort_selection(was_upgrade),
                Flavor::Sync(ref p) => return p.abort_selection(),
            };
            let new_port = match result { Ok(b) => return b, Err(p) => p };
            was_upgrade = true;
            unsafe {
                mem::swap(self.inner_mut(),
                          new_port.inner_mut());
            }
        }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> { self.rx.recv().ok() }
}

impl<'a, T> Iterator for TryIter<'a, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> { self.rx.try_recv().ok() }
}

impl<'a, T> IntoIterator for &'a Receiver<T> {
    type Item = T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Iter<'a, T> { self.iter() }
}

impl<T> Iterator for IntoIter<T> {
    type Item = T;
    fn next(&mut self) -> Option<T> { self.rx.recv().ok() }
}

impl <T> IntoIterator for Receiver<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    fn into_iter(self) -> IntoIter<T> {
        IntoIter { rx: self }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        match *unsafe { self.inner() } {
            Flavor::Oneshot(ref p) => p.drop_port(),
            Flavor::Stream(ref p) => p.drop_port(),
            Flavor::Shared(ref p) => p.drop_port(),
            Flavor::Sync(ref p) => p.drop_port(),
        }
    }
}

impl<T> fmt::Debug for Receiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Receiver {{ .. }}")
    }
}

impl<T> fmt::Debug for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        "SendError(..)".fmt(f)
    }
}

impl<T> fmt::Display for SendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        "sending on a closed channel".fmt(f)
    }
}

impl<T: Send> error::Error for SendError<T> {
    fn description(&self) -> &str {
        "sending on a closed channel"
    }

    fn cause(&self) -> Option<&error::Error> {
        None
    }
}

impl<T> fmt::Debug for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            TrySendError::Full(..) => "Full(..)".fmt(f),
            TrySendError::Disconnected(..) => "Disconnected(..)".fmt(f),
        }
    }
}

impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            TrySendError::Full(..) => {
                "sending on a full channel".fmt(f)
            }
            TrySendError::Disconnected(..) => {
                "sending on a closed channel".fmt(f)
            }
        }
    }
}

impl<T: Send> error::Error for TrySendError<T> {

    fn description(&self) -> &str {
        match *self {
            TrySendError::Full(..) => {
                "sending on a full channel"
            }
            TrySendError::Disconnected(..) => {
                "sending on a closed channel"
            }
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        None
    }
}

impl fmt::Display for RecvError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        "receiving on a closed channel".fmt(f)
    }
}

impl error::Error for RecvError {

    fn description(&self) -> &str {
        "receiving on a closed channel"
    }

    fn cause(&self) -> Option<&error::Error> {
        None
    }
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            TryRecvError::Empty => {
                "receiving on an empty channel".fmt(f)
            }
            TryRecvError::Disconnected => {
                "receiving on a closed channel".fmt(f)
            }
        }
    }
}

impl error::Error for TryRecvError {

    fn description(&self) -> &str {
        match *self {
            TryRecvError::Empty => {
                "receiving on an empty channel"
            }
            TryRecvError::Disconnected => {
                "receiving on a closed channel"
            }
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        None
    }
}

impl fmt::Display for RecvTimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            RecvTimeoutError::Timeout => {
                "timed out waiting on channel".fmt(f)
            }
            RecvTimeoutError::Disconnected => {
                "channel is empty and sending half is closed".fmt(f)
            }
        }
    }
}

impl error::Error for RecvTimeoutError {
    fn description(&self) -> &str {
        match *self {
            RecvTimeoutError::Timeout => {
                "timed out waiting on channel"
            }
            RecvTimeoutError::Disconnected => {
                "channel is empty and sending half is closed"
            }
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        None
    }
}


pub fn stress_factor() -> usize {
    match std::env::var("RUST_TEST_STRESS") {
        Ok(val) => val.parse().unwrap(),
        Err(..) => 1,
    }
}

fn stress_recv_timeout_shared() {
    let (tx, rx) = channel();
    let stress = stress_factor() + 10;

    for i in 0..stress {
        let tx = tx.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(i as u64 * 10));
            tx.send(1usize).unwrap();
            println!("s:{}", i);
        });
    }

    drop(tx);

    let mut recv_count = 0;
    loop {
        match rx.recv_timeout(Duration::from_millis(10)) { // <=10都会出问题
            Ok(n) => {
                assert_eq!(n, 1usize);
                recv_count += 1;
                println!("r:{}", recv_count);
            }
            Err(RecvTimeoutError::Timeout) => {
                println!("timeout");
                continue;},
            Err(RecvTimeoutError::Disconnected) => {
                println!("disconnect");
                break;
            },
        }
    }

    assert_eq!(recv_count, stress);
}

fn main()
{
    xpsupport_sys::init();
    stress_recv_timeout_shared();
}
