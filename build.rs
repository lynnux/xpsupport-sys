#![allow(unused_must_use)]

// use std::fs;
// use std::path;

fn main() {
    // use std::env;
    // let out_dir = env::var("OUT_DIR").unwrap();
    // let path = path::Path::new(&out_dir);
    // let deps_dir = path.parent().unwrap().parent().unwrap().parent().unwrap().join("deps");
    
    // let new_name = deps_dir.clone().join("xpsupport.dll");

    // if !new_name.exists()
    // {
    //     let r = fs::read_dir(&deps_dir);
    //     match r
    //     { 
    //         Ok(_) =>
    //         {
    //             println!("cargo:rustc-link-search=11");

    //             for entry in r.unwrap() {
    //                 if let Ok(entry) = entry
    //                 {
    //                     let path = entry.path();

    //                     if path.is_file()
    //                         && path.extension().unwrap() == "dll"
    //                         && path.file_name().unwrap().to_str().unwrap().starts_with("xpsupport-"){
    //                             println!("cargo:rustc-link-search={}", path.to_str().unwrap());
    //                             println!("cargo:rustc-link-search={}", new_name.to_str().unwrap());

    //                             fs::rename(path, &new_name);
    //                             break;
    //                         }
    //                 }
    //             }
    //         },
    //         Err(e)=>
    //         {
    //             println!("{:?}", e);
    //         }
    //     }
    // }

    // println!("cargo:rustc-link-search={}", deps_dir.to_str().unwrap());

    // // by the way, copy xpsupport.dll to target directory
    // let copy_path = deps_dir.clone().parent().unwrap().join("xpsupport.dll");
    // if !copy_path.exists()
    // {
    //     fs::copy(new_name, copy_path);
    // }

    println!("cargo:rustc-flags=-l xpsupport");
}
