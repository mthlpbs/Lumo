// Shared by the before/after Windows engine-cost harnesses. No wall-clock CPU
// approximation: QueryProcessCycleTime counts actual process CPU cycles.
#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct Memory {
    cb:u32,faults:u32,peak_working:usize,working:usize,
    peak_paged:usize,paged:usize,peak_nonpaged:usize,nonpaged:usize,
    pagefile:usize,peak_pagefile:usize,private:usize,
}
#[cfg(windows)]
#[link(name="kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess()->*mut std::ffi::c_void;
    fn QueryProcessCycleTime(process:*mut std::ffi::c_void,cycles:*mut u64)->i32;
}
#[cfg(windows)]
#[link(name="psapi")]
unsafe extern "system" {
    fn GetProcessMemoryInfo(process:*mut std::ffi::c_void,memory:*mut Memory,size:u32)->i32;
}
#[cfg(windows)]
fn memory()->Memory {
    let mut out=Memory{cb:std::mem::size_of::<Memory>()as u32,..Default::default()};
    assert_ne!(unsafe{GetProcessMemoryInfo(GetCurrentProcess(),&mut out,out.cb)},0);out
}
#[cfg(windows)]
fn cycles()->u64 {let mut n=0;assert_ne!(unsafe{QueryProcessCycleTime(GetCurrentProcess(),&mut n)},0);n}
#[cfg(windows)]
fn main(){
    let root=std::path::PathBuf::from(std::env::args().nth(1).expect("protection data directory"));
    let before=memory();let begin=cycles();
    let lists=["easylist.txt","easyprivacy.txt"].into_iter().map(|n|std::fs::read_to_string(root.join(n)).unwrap()).collect();
    let resources=std::fs::read_to_string(root.join("resources.json")).unwrap();
    let engine=load_engine(lists,resources);
    let compiled_cycles=cycles()-begin;
    for _ in 0..200 {std::hint::black_box(engine.blocked("https://googleads.g.doubleclick.net/pagead/ads","https://news.example.com/",engine::SCRIPT));}
    let begin=cycles();let mut blocked=0;
    for i in 0..10000 {let url=if i%2==0{"https://googleads.g.doubleclick.net/pagead/ads"}else{"https://example.com/content/article.js"};blocked+=usize::from(std::hint::black_box(engine.blocked(url,"https://news.example.com/",engine::SCRIPT)));}
    let query_cycles=cycles()-begin;let active=memory();drop(engine);let closed=memory();
    println!("{{\"baseline_private\":{},\"baseline_working\":{},\"active_private\":{},\"active_working\":{},\"closed_private\":{},\"closed_working\":{},\"compile_cycles\":{},\"query_cycles_10000\":{},\"blocked\":{}}}",before.private,before.working,active.private,active.working,closed.private,closed.working,compiled_cycles,query_cycles,blocked);
}
#[cfg(not(windows))]
fn main(){eprintln!("This harness measures Windows process memory and CPU cycles.");}
