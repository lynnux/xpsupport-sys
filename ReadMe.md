## ReadMe
From version `0.2` of [xpsupport](https://github.com/lynnux/xpsupport), there no longer need this crate.
I just use this repo to test libstd/sync for xpsupport (latest: 2019 Nov 1).
## Usage
Build with command: cargo test.
## Testing result
```
test result: ok. 148 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```
Only [`mpsc::stress_recv_timeout_shared`](https://github.com/lynnux/xpsupport-sys/blob/master/test/mpsc/mod.rs#L463) seems deadlock, other 148 are all passed! 
You may consider [parking_lot](https://github.com/Amanieu/parking_lot) crate as the sync library or [spin](https://github.com/mvdnes/spin-rs), they both support XP.
