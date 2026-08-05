use crate::mm::MM;

mod bitmap;
mod gc_sync;
mod mm;
mod object;
mod state;
mod sync;

fn main() {
    println!("Hello, world!");
    let mut mm = MM::new(128 * 1024 * 1024).unwrap();

    let mut ctx = mm::Context::new();

    let (bytes, page_id) = ctx.alloc(&mm, 8291).unwrap();
    println!("Bytes: 0x{:16x} from page {page_id:#6}", bytes.addr());

    let (bytes, page_id) = ctx.alloc(&mm, 8291).unwrap();
    println!("Bytes: 0x{:16x} from page {page_id:#6}", bytes.addr());

    let (bytes, page_id) = ctx.alloc(&mm, 8291).unwrap();
    println!("Bytes: 0x{:16x} from page {page_id:#6}", bytes.addr());

    let mut ctx = mm::Context::new();
    let (bytes, page_id) = ctx.alloc(&mm, 8291).unwrap();
    println!("Bytes: 0x{:16x} from page {page_id:#6}", bytes.addr());

    ctx.flush_local_buf();

    let mm2 = unsafe { mm.remap_and_clear() }.unwrap();
    let (bytes, page_id) = ctx.alloc(&mm, 8291).unwrap();
    println!(
        "Bytes after cleared: 0x{:16x} from page {page_id:#6}",
        bytes.addr()
    );
}
