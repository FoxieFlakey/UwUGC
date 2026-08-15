// Mainly just contain quirks atleast on my system UFFD_MOVE
// is undereporting? im stil figuring where it happen or whether
// its nature of me compiling with bleeding edge Clang or not.
//
// For now this is slow path for "fixing"

use std::ffi::c_void;

pub fn uffd_try_fix_moved(
    dest: *mut c_void,
    moved_reported: &mut usize,
    moved_intended: usize,
) {
    assert!(moved_intended.is_multiple_of(page_size::get()), "Moved intended must be multiple of page size");
    assert!(dest.addr().is_multiple_of(page_size::get()), "Dest pointer must be multiple of page size");
    if moved_intended == *moved_reported {
        return;
    }

    if !moved_reported.is_multiple_of(page_size::get()) {
        eprintln!("WEIRD: UFFD report non multiple of page size is moved ({moved_reported} excess of {}), rounding up", *moved_reported % page_size::get());
        *moved_reported = moved_reported.next_multiple_of(page_size::get());
    }

    let nr_to_check = moved_intended - *moved_reported;
    let mut vector = Vec::new();
    vector.resize_with(nr_to_check, || 0u8);

    let check_start = dest.wrapping_byte_add(*moved_reported);

    // SAFETY: The resulting buffer is same size as needed
    nix::errno::Errno::result(unsafe { nix::libc::mincore(check_start, nr_to_check, vector.as_mut_ptr()) }).unwrap();

    let reported_pages = *moved_reported / page_size::get();
    let mut actual_moved_pages = reported_pages;
    let intended_pages = moved_intended / page_size::get();
    
    for is_present in vector.into_iter().map(|x| (x & 1) != 0) {
        if !is_present {
            // We updated the moved_report to be correct
            break;
        }
        *moved_reported += page_size::get();
        actual_moved_pages += 1;
    }

    eprintln!("QUIRK FIX: Underreporting of UFFD page triggered");
    eprintln!("QUIRK FIX: UFFD reported only {} pages moved, but {} pages actually moved, the intended was {} pages", reported_pages, actual_moved_pages, intended_pages);
}

