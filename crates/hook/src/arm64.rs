//! ARM64 helpers for macOS/iOS image scanning, instruction decoding, and
//! safe process memory reads.

#[cfg(target_vendor = "apple")]
use std::ffi::c_int;
use std::ops::Range;

use crate::image::Image;

#[cfg(target_vendor = "apple")]
unsafe extern "C" {
   static mach_task_self_: u32;
   fn vm_read_overwrite(
      target_task: u32,
      address: usize,
      size: usize,
      data: usize,
      outsize: *mut usize,
   ) -> c_int;
}

/// Copies up to `len` bytes of this process's memory at `addr`, or `None` when
/// the start is unmapped.
#[cfg(target_vendor = "apple")]
#[must_use]
pub fn read_memory(addr: usize, len: usize) -> Option<Vec<u8>> {
   if addr == 0 || len == 0 {
      return None;
   }
   let mut out = vec![0u8; len];
   let mut read = 0usize;
   // SAFETY: `out` is valid for `len` bytes and `vm_read_overwrite` fails
   // instead of faulting on unmapped pages.
   let status = unsafe {
      vm_read_overwrite(
         mach_task_self_,
         addr,
         len,
         out.as_mut_ptr() as usize,
         &raw mut read,
      )
   };
   if status != 0 || read == 0 {
      return None;
   }
   out.truncate(read);
   Some(out)
}

/// A code site in an ARM64 image that loads or references a target address.
#[derive(Clone, Debug)]
pub struct CodeRef {
   pub addr: usize,
   pub target: usize,
   pub function: usize,
}

/// Target address of an `adrp xN, page` instruction at `pc`, returning `(rd, page_addr)`.
#[must_use]
pub const fn decode_adrp(insn: u32, pc: usize) -> Option<(u32, usize)> {
   if insn & 0x9F00_0000 != 0x9000_0000 {
      return None;
   }
   let rd = insn & 0x1F;
   let pages = ((((insn >> 5) & 0x7_FFFF) << 2) | ((insn >> 29) & 3)).cast_signed() << 11 >> 11;
   let page = (pc & !0xFFF).wrapping_add_signed((pages as isize) << 12);
   Some((rd, page))
}

/// Target address of an `adrp xN, page; add xN, xN, #imm12` pair at `pc`.
#[must_use]
pub const fn decode_adrp_add(adrp: u32, add: u32, pc: usize) -> Option<(u32, usize)> {
   let Some((rd, page)) = decode_adrp(adrp, pc) else {
      return None;
   };
   // 64-bit `add xd, xn, #imm12` (unshifted, shift == 0)
   if add & 0xFFC0_0000 != 0x9100_0000 {
      return None;
   }
   let src = (add >> 5) & 0x1F;
   let dst = add & 0x1F;
   if src != rd {
      return None;
   }
   let imm12 = ((add >> 10) & 0xFFF) as usize;
   Some((dst, page.wrapping_add(imm12)))
}

/// Target address of a `bl #imm26` or `b #imm26` instruction at `pc`.
#[must_use]
pub const fn decode_branch26(insn: u32, pc: usize) -> Option<(bool, usize)> {
   let op = insn & 0xFC00_0000;
   if op != 0x9400_0000 && op != 0x1400_0000 {
      return None;
   }
   let is_link = op == 0x9400_0000;
   let imm26 = (insn & 0x03FF_FFFF).cast_signed() << 6 >> 6;
   let target = pc.wrapping_add_signed((imm26 as isize) << 2);
   Some((is_link, target))
}

/// Scans executable segments for `adrp + add` pairs that materialize an address
/// inside `range`.
#[must_use]
pub fn adrp_add_refs(image: &Image<'_>, range: Range<usize>) -> Vec<CodeRef> {
   let mut refs = Vec::new();
   for seg in image.segments.iter().filter(|seg| seg.executable) {
      let words = seg.bytes.as_chunks::<4>().0;
      for i in 0..words.len().saturating_sub(1) {
         let adrp = u32::from_le_bytes(words[i]);
         let pc = seg.addr + i * 4;
         let Some((rd, page)) = decode_adrp(adrp, pc) else {
            continue;
         };
         // Quick page check before checking add.
         if range.end <= page || range.start >= page.saturating_add(0x1000) {
            continue;
         }
         // Check the next few instructions in case another instruction was scheduled in between.
         for j in 1..=4.min(words.len() - 1 - i) {
            let next = u32::from_le_bytes(words[i + j]);
            if next & 0xFFC0_0000 == 0x9100_0000 && ((next >> 5) & 0x1F) == rd {
               let imm12 = ((next >> 10) & 0xFFF) as usize;
               let target = page.wrapping_add(imm12);
               if range.contains(&target) {
                  let function = image.function_bounds(pc).map_or(pc, |(start, _)| start);
                  refs.push(CodeRef {
                     addr: pc,
                     target,
                     function,
                  });
               }
               break;
            }
         }
      }
   }
   refs
}

/// Scans executable segments for `bl` or `b` instructions targeting `range`.
#[must_use]
pub fn branch_refs(image: &Image<'_>, range: Range<usize>) -> Vec<CodeRef> {
   let mut refs = Vec::new();
   for seg in image.segments.iter().filter(|seg| seg.executable) {
      let words = seg.bytes.as_chunks::<4>().0;
      for (i, &word) in words.iter().enumerate() {
         let insn = u32::from_le_bytes(word);
         let pc = seg.addr + i * 4;
         if let Some((_, target)) = decode_branch26(insn, pc)
            && range.contains(&target)
         {
            let function = image.function_bounds(pc).map_or(pc, |(start, _)| start);
            refs.push(CodeRef {
               addr: pc,
               target,
               function,
            });
         }
      }
   }
   refs
}
