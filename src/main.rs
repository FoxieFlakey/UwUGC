use crate::mm::MM;

mod bitmap;
mod mm;
mod object;

fn main() {
    println!("Hello, world!");
    let mm = MM::new(128 * 1024 * 1024).unwrap();

    let mut ctx = mm::Context::new();

    let bytes = ctx.alloc(&mm, 8291);
    println!("Bytes: 0x{:16x}", bytes.unwrap().addr());

    let bytes = ctx.alloc(&mm, 8291);
    println!("Bytes: 0x{:16x}", bytes.unwrap().addr());

    let bytes = ctx.alloc(&mm, 8291);
    println!("Bytes: 0x{:16x}", bytes.unwrap().addr());

    let mut ctx = mm::Context::new();
    let bytes = ctx.alloc(&mm, 8291);
    println!("Bytes: 0x{:16x}", bytes.unwrap().addr());
}
