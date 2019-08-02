// src/lib.rs

use std::error::Error;
use std::ffi;
use std::mem;
use std::sync;

use lazy_static::lazy_static;
use libc;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct bpf_insn_t {
    pub code: libc::c_ushort,
    pub jt: libc::c_uchar,
    pub jf: libc::c_uchar,
    pub k: libc::c_uint,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct bpf_program_t {
    pub bf_len: libc::c_uint,
    pub bf_insns: *mut bpf_insn_t,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct bpf_args_t {
    pub pkt: *const libc::c_uchar,
    pub wirelen: libc::size_t,
    pub buflen: libc::size_t,
    pub mem: *mut libc::c_uint,
    pub arg: *mut ffi::c_void,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct bpf_ctx_t {
    pub copfuncs: *const ffi::c_void,
    pub nfuncs: libc::size_t,
    pub extwords: libc::size_t,
    pub preinited: libc::c_uint,
}

type bpfjit_func_t =
    Option<unsafe extern "C" fn(ctx: *const bpf_ctx_t, args: *mut bpf_args_t) -> libc::c_uint>;

#[link(name = "pcap")]
extern "C" {
    #[link_name = "pcap_open_dead"]
    fn pcap_open_dead(linktype: libc::c_int, snaplen: libc::c_int) -> *mut ffi::c_void;

    #[link_name = "pcap_compile"]
    fn pcap_compile(
        p: *mut ffi::c_void,
        fp: *mut bpf_program_t,
        str: *const libc::c_char,
        optimize: libc::c_int,
        netmask: libc::c_uint,
    ) -> libc::c_int;

    #[link_name = "pcap_close"]
    fn pcap_close(p: *mut ffi::c_void);

    #[link_name = "pcap_geterr"]
    fn pcap_geterr(p: *mut ffi::c_void) -> *const libc::c_char;
}

extern "C" {
    #[link_name = "bpfjit_generate_code"]
    fn bpfjit_generate_code(
        ctx: *const bpf_ctx_t,
        insns: *const bpf_insn_t,
        user: libc::size_t,
    ) -> bpfjit_func_t;

    #[link_name = "bpfjit_free_code"]
    fn bpfjit_free_code(func: bpfjit_func_t);
}

lazy_static! {
    static ref BIGLOCK: sync::Mutex<u8> = sync::Mutex::new(0);
}

pub struct BpfJit {
    pub prog: bpf_program_t,
    pub ctx: *const bpf_ctx_t,
    pub cb: bpfjit_func_t,
}

impl BpfJit {
    pub fn new(filter: &str) -> Result<Self, Box<Error>> {
        BpfJit::new_ethernet(filter)
    }

    pub fn new_ethernet(filter: &str) -> Result<Self, Box<Error>> {
        unsafe {
            let mut result: BpfJit = mem::zeroed();

            let lock = BIGLOCK.lock()?; // pcap_compile() in libpcap < 1.8 is not thread-safe

            let pcap = pcap_open_dead(1 /* LINKTYPE_ETHERNET */, 65535);
            let compiled = pcap_compile(
                pcap,
                &mut result.prog,
                ffi::CString::new(filter)?.as_ptr(),
                1,
                0xffffffff,
            );
            pcap_close(pcap);

            drop(lock);

            if compiled != 0 {
                return Err(Box::from(format!(
                    "could not compile cBPF expression: {}",
                    ffi::CStr::from_ptr(pcap_geterr(pcap)).to_str().unwrap()
                )));
            }

            result.cb = bpfjit_generate_code(
                result.ctx,
                result.prog.bf_insns,
                result.prog.bf_len as libc::size_t,
            );
            if result.cb.is_none() {
                return Err(Box::from("could not JIT cBPF expression"));
            }

            Ok(result)
        }
    }

    pub fn new_ip(filter: &str) -> Result<Self, Box<Error>> {
        unsafe {
            let mut result: BpfJit = mem::zeroed();

            let lock = BIGLOCK.lock()?; // pcap_compile() in libpcap < 1.8 is not thread-safe

            let pcap = pcap_open_dead(12 /* LINKTYPE_RAW */, 65535);
            let compiled = pcap_compile(
                pcap,
                &mut result.prog,
                ffi::CString::new(filter)?.as_ptr(),
                1,
                0xffffffff,
            );
            pcap_close(pcap);

            drop(lock);

            if compiled != 0 {
                return Err(Box::from(format!(
                    "could not compile cBPF expression: {}",
                    ffi::CStr::from_ptr(pcap_geterr(pcap)).to_str().unwrap()
                )));
            }

            result.cb = bpfjit_generate_code(
                result.ctx,
                result.prog.bf_insns,
                result.prog.bf_len as libc::size_t,
            );
            if result.cb.is_none() {
                return Err(Box::from("could not JIT cBPF expression"));
            }

            Ok(result)
        }
    }

    pub fn matches(&self, data: &[u8]) -> bool {
        unsafe {
            let mut bpf_args: bpf_args_t = mem::zeroed();
            bpf_args.pkt = data.as_ptr();
            bpf_args.wirelen = data.len();
            bpf_args.buflen = data.len();

            self.cb.unwrap()(self.ctx, &mut bpf_args) != 0
        }
    }

    pub fn get_bpf_raw(&self) -> bpf_program_t {
        self.prog
    }

    pub fn get_bpf(&self) -> bpfjit_func_t {
        self.cb
    }

    pub fn print_bpf(&self) {
        unsafe {
            let n = self.prog.bf_len;

            let insn = self.prog.bf_insns;
            for i in 0..n {
                println!(
                    "{ 0x{:x}, {}, {}, 0x{:} }, \n",
                    insn.code, insn.jt, insn.jf, isns.k
                );
                insn += 1;
            }
        }
    }
}

impl Clone for BpfJit {
    fn clone(&self) -> Self {
        unsafe {
            let mut result: BpfJit = mem::zeroed();

            result.prog = self.prog;

            result.cb = bpfjit_generate_code(
                result.ctx,
                result.prog.bf_insns,
                result.prog.bf_len as libc::size_t,
            );
            if result.cb.is_none() {
                panic!("could not JIT cBPF expression"); // we already JIT'ed the same program before, so this should never happen
            }

            result
        }
    }
}

impl Drop for BpfJit {
    fn drop(&mut self) {
        unsafe {
            if self.cb.is_some() {
                bpfjit_free_code(self.cb);
            }
        }
    }
}

unsafe impl Send for BpfJit {}

unsafe impl Sync for BpfJit {}
