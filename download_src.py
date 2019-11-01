#!/usr/bin/env python
# -*- coding: utf-8 -*-

import os
import sys
import requests

all = [
    "barrier.rs",
    "condvar.rs",
    "mutex.rs",
    "mod.rs",
    "once.rs",
    "rwlock.rs",
    "mpsc/blocking.rs",
    "mpsc/cache_aligned.rs",
    "mpsc/mod.rs",
    "mpsc/mpsc_queue.rs",
    "mpsc/oneshot.rs",
    "mpsc/shared.rs",
    "mpsc/spsc_queue.rs",
    "mpsc/stream.rs",
    "mpsc/sync.rs"
]
def download():
    import socket
    socket.setdefaulttimeout(10)

    for f in all:
        if os.path.exists("download/"+f):
            continue
        print(f)
        try:
            url = "https://raw.githubusercontent.com/rust-lang/rust/master/src/libstd/sync/" + f
            r = requests.get(url, proxies={'http': '127.0.0.1:3128'}) # need proxy in china!
            with open("download/"+f,'wb') as f:
                f.write(r.content)
        except:
            pass

def main():
    try:
        os.mkdir("download")
    except:
        pass
    try:
        os.mkdir("download/mpsc")
    except:
        pass    
    download()

if __name__ == "__main__":
    try:
        main()
    except:
        import traceback
        traceback.print_exc()
        input()
