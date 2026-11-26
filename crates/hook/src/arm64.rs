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
#[derive(Clone, Copy, Debug)]
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

/// Every `bl` target in `[start, start + len)`, following a 1-hop outlined
/// tail-branch `b` thunk when present.
#[must_use]
pub fn calls_in(image: &Image<'_>, start: usize, len: usize) -> Vec<(usize, usize)> {
   let Some(bytes) = image.read(start, len) else {
      return Vec::new();
   };
   let mut out = Vec::new();
   for (i, chunk) in bytes.as_chunks::<4>().0.iter().enumerate() {
      let pc = start + i * 4;
      let insn = u32::from_le_bytes(*chunk);
      if let Some((true, mut target)) = decode_branch26(insn, pc) {
         if let Some(thunk) = image.read(target, 16) {
            for (j, tchunk) in thunk.as_chunks::<4>().0.iter().enumerate() {
               let tins = u32::from_le_bytes(*tchunk);
               if tins == 0xD65F_03C0 {
                  break;
               }
               if let Some((false, real)) = decode_branch26(tins, target + j * 4) {
                  target = real;
                  break;
               }
            }
         }
         out.push((pc, target));
      }
   }
   out
}

/// A `DebugStruct::field` call decoded from an ARM64 derived `Debug::fmt`.
#[derive(Clone, Copy, Debug)]
pub struct DebugField {
   pub name:     usize,
   pub name_len: usize,
   pub offset:   usize,
   pub vtable:   Option<usize>,
}

/// Decodes `DebugStruct::field(&mut ds, name, &self.field)` calls from an
/// ARM64 `Debug::fmt(self: &T, f: &mut Formatter)` body.
#[must_use]
pub fn debug_fields(code: &[u8], start: usize) -> Vec<DebugField> {
   let mut adrp_page: [Option<usize>; 32] = [None; 32];
   let mut reg_addr: [Option<usize>; 32] = [None; 32];
   let mut reg_imm: [Option<usize>; 32] = [None; 32];
   let mut reg_self: [Option<usize>; 32] = [None; 32];
   reg_self[0] = Some(0);
   let mut saved_self: std::collections::HashMap<usize, [Option<usize>; 32]> =
      std::collections::HashMap::new();
   let mut out = Vec::new();

   for (i, chunk) in code.as_chunks::<4>().0.iter().enumerate() {
      let pc = start + i * 4;
      if let Some(saved) = saved_self.get(&pc) {
         reg_self = *saved;
      }
      let insn = u32::from_le_bytes(*chunk);

      // Conditional branches (`b.cond`, `cbz`/`cbnz`, `tbz`/`tbnz`): remember
      // self-relative register state at forward targets so fallthrough
      // pre-indexed loads do not corrupt branch targets.
      let branch_target =
         if insn & 0xFF00_0010 == 0x5400_0000 || insn & 0x7E00_0000 == 0x3400_0000 {
            let imm19 = (((insn >> 5) & 0x7_FFFF).cast_signed() << 13 >> 13) as isize;
            Some(pc.wrapping_add_signed(imm19 << 2))
         } else if insn & 0x7E00_0000 == 0x3600_0000 {
            let imm14 = (((insn >> 5) & 0x3FFF).cast_signed() << 18 >> 18) as isize;
            Some(pc.wrapping_add_signed(imm14 << 2))
         } else {
            None
         };
      if let Some(target) = branch_target {
         if target > pc {
            saved_self.insert(target, reg_self);
         }
         continue;
      }

      // `mov xD, xS` (`orr xD, xzr, xS` = 0xAA00_03E0) or `mov wD, wS` (0x2A00_03E0)
      if insn & 0x7FE0_FFE0 == 0x2A00_03E0 {
         let rd = (insn & 0x1F) as usize;
         let rm = ((insn >> 16) & 0x1F) as usize;
         adrp_page[rd] = adrp_page[rm];
         reg_addr[rd] = reg_addr[rm];
         reg_imm[rd] = reg_imm[rm];
         reg_self[rd] = if insn & 0x8000_0000 != 0 {
            reg_self[rm]
         } else {
            None
         };
         continue;
      }

      // Pre-indexed scalar or SIMD load/store `[xN, #simm9]!` (0x3800_0C00 / 0x3B00_0C00)
      if insn & 0x3B00_0C00 == 0x3800_0C00 {
         let rn = ((insn >> 5) & 0x1F) as usize;
         let simm9 = (((insn >> 12) & 0x1FF).cast_signed() << 23 >> 23) as isize;
         if let Some(off) = reg_self[rn] {
            reg_self[rn] = Some(off.wrapping_add_signed(simm9));
         }
         continue;
      }

      if let Some((rd, page)) = decode_adrp(insn, pc) {
         let rd = rd as usize;
         adrp_page[rd] = Some(page);
         reg_addr[rd] = None;
         reg_self[rd] = None;
         continue;
      }

      // 64-bit `add xD, xN, #imm12` (unshifted)
      if insn & 0xFFC0_0000 == 0x9100_0000 {
         let rd = (insn & 0x1F) as usize;
         let rn = ((insn >> 5) & 0x1F) as usize;
         let imm12 = ((insn >> 10) & 0xFFF) as usize;
         if let Some(page) = adrp_page[rn] {
            reg_addr[rd] = Some(page.wrapping_add(imm12));
         }
         reg_self[rd] = reg_self[rn].map(|off| off.wrapping_add(imm12));
         continue;
      }

      // `movz wD, #imm16` (32-bit, shift 0): 0x5280_0000
      if insn & 0xFFE0_0000 == 0x5280_0000 {
         let rd = (insn & 0x1F) as usize;
         let imm16 = ((insn >> 5) & 0xFFFF) as usize;
         reg_imm[rd] = Some(imm16);
         reg_self[rd] = None;
         continue;
      }

      if let Some((true, _)) = decode_branch26(insn, pc) {
         if let (Some(name), Some(name_len), Some(offset)) = (reg_addr[1], reg_imm[2], reg_self[3])
            && name_len > 0
            && name_len <= 64
         {
            out.push(DebugField {
               name,
               name_len,
               offset,
               vtable: reg_addr[4],
            });
         }
         for r in 0..=18 {
            adrp_page[r] = None;
            reg_addr[r] = None;
            reg_imm[r] = None;
            reg_self[r] = None;
         }
      }
   }
   out
}
