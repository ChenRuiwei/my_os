use core::arch::asm;

use bit_field::BitField;
use riscv::register::sstatus::{FS, SPP};

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct Sstatus {
    bits: usize,
}

impl Sstatus {
    #[inline]
    pub fn fs(&self) -> FS {
        match self.bits.get_bits(13..15) {
            0 => FS::Off,
            1 => FS::Initial,
            2 => FS::Clean,
            3 => FS::Dirty,
            _ => unreachable!(),
        }
    }
    #[inline]
    pub fn set_spie(&mut self, val: bool) {
        self.bits.set_bit(5, val);
    }
    #[inline]
    pub fn set_sie(&mut self, val: bool) {
        self.bits.set_bit(1, val);
    }
    #[inline]
    pub fn set_spp(&mut self, spp: SPP) {
        self.bits.set_bit(8, spp == SPP::Supervisor);
    }
    #[inline]
    pub fn set_fs(&mut self, fs: FS) {
        let v: u8 = unsafe { core::mem::transmute(fs) };
        self.bits.set_bits(13..15, v as usize);
    }
}

pub fn read() -> Sstatus {
    let bits: usize;
    unsafe {
        asm!("csrr {}, sstatus", out(reg) bits);
    }
    Sstatus { bits }
}
