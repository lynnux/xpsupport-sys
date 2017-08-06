#![feature(once_poison)]
#![feature(const_fn)]
#![feature(box_syntax)]

#[cfg(test)]
extern crate rand;

extern crate xpsupport_sys;

#[test]
fn test_init()
{
    xpsupport_sys::init();
}

mod mpsc;
mod barrier;
mod condvar;
mod mutex;
mod once;
mod rwlock;
