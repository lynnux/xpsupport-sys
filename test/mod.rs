#![feature(once_poison)]
#![feature(const_fn)]
#![feature(box_syntax)]

#[cfg(test)]
extern crate rand;

extern crate xpsupport;

#[test]
fn test_init()
{
    xpsupport::init();
}

mod mpsc;
mod barrier;
mod condvar;
mod mutex;
mod once;
mod rwlock;
