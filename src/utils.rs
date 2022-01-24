use std::io::Read;
use std::mem::{size_of, MaybeUninit};

pub(crate) fn get_random_number<T>() -> MaybeUninit<T> {
    let mut file =
        std::fs::File::open("/dev/urandom").expect("No /dev/urandom on an UNIX system?!");
    let mut value: MaybeUninit<T> = MaybeUninit::zeroed();

    #[cfg(target_family = "unix")]
    {
        // SAFETY: we own the memory
        let slice =
            unsafe { std::slice::from_raw_parts_mut(value.as_mut_ptr().cast(), size_of::<T>()) };

        file.read(slice)
            .expect("Could not read from /dev/urandom?!");
    }

    value
}
