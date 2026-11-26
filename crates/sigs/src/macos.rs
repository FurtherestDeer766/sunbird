//! Locators for macOS Desktop Phoenix (`Discord Phoenix.app/Contents/MacOS/Discord`, arm64).

use phoenix_hook::{image::Image, pattern::Pattern};

use crate::{Found, one};

/// `should_enforce_client_access_gate` in `libdiscord_ops_surfaces::client_access`.
///
/// The function starts 8 bytes before this body (`mov w8, #imm; add x8, x0, x8`),
/// loads the session state (`cmp x9, #2`), user id (`cmn x8, #1`), and staff flag
/// (`ldrb w8, [x0, #imm]; eor w8, w8, #1`). Patching its entry to `mov w0, #0; ret`
/// makes every `ClientAccessGate` evaluator return `None` (no gate).
const ACCESS_GATE: Pattern = Pattern::new(
   "09 01 40 F9 3F 09 00 F1 E0 00 00 54 08 ?? ?? F9 1F 05 00 B1 80 00 00 54 08 ?? ?? 39 08 01 00 \
    52 02 00 00 14 08 00 80 52 00 01 00 12 C0 03 5F D6",
);

/// Entry point of `should_enforce_client_access_gate`, which opens the gate
/// when patched to return `false` (`mov w0, #0; ret`).
pub fn access_gate(image: &Image<'_>) -> Found<usize> {
   let body = one("macos access gate", &image.scan(ACCESS_GATE))?;
   let entry = body
      .checked_sub(8)
      .ok_or("macos access gate body too close to start of segment")?;
   if image.is_code(entry) {
      Ok(entry)
   } else {
      Err(format!("macos access gate entry {entry:#x} is not code"))
   }
}
