// run-pass
// ignore-wasm32-bare seems not important to test here

#![feature(intrinsics)]

mod rusti {
    extern "rust-intrinsic" {
        pub fn pref_align_of<T>() -> usize;
        #[rustc_safe_intrinsic]
        pub fn min_align_of<T>() -> usize;
    }
}

#[cfg(any(target_os = "android",
          target_os = "dragonfly",
          target_os = "emscripten",
          target_os = "freebsd",
          target_os = "fuchsia",
          target_os = "illumos",
          target_os = "linux",
          target_os = "macos",
          target_os = "netbsd",
          target_os = "openbsd",
          target_os = "solaris",
          target_os = "vxworks"))]
mod m {
    #[cfg(target_arch = "x86")]
    pub fn main() {
        unsafe {
            assert_eq!(::rusti::pref_align_of::<u64>(), 8);
            assert_eq!(::rusti::min_align_of::<u64>(), 4);
        }
    }

    #[cfg(not(target_arch = "x86"))]
    pub fn main() {
        unsafe {
            assert_eq!(::rusti::pref_align_of::<u64>(), 8);
            assert_eq!(::rusti::min_align_of::<u64>(), 8);
        }
    }
}

#[cfg(target_env = "sgx")]
mod m {
    #[cfg(target_arch = "x86_64")]
    pub fn main() {
        unsafe {
            assert_eq!(::rusti::pref_align_of::<u64>(), 8);
            assert_eq!(::rusti::min_align_of::<u64>(), 8);
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "uefi"))]
mod m {
    pub fn main() {
        unsafe {
            assert_eq!(::rusti::pref_align_of::<u64>(), 8);
            assert_eq!(::rusti::min_align_of::<u64>(), 8);
        }
    }
}

fn main() {
    m::main();
}
