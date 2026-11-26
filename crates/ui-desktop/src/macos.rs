//! macOS desktop backend through standard system utilities (`pbcopy`, `open`,
//! `osascript`), so the crate links nothing beyond libc.

use std::{
   io::Write as _,
   path::PathBuf,
   process::{Command, Stdio},
};

use crate::{DesktopHost, Error, spawn_reaped};

const APP_BROWSERS: [&str; 5] = [
   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
   "/Applications/Chromium.app/Contents/MacOS/Chromium",
   "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
   "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
   "/Applications/Arc.app/Contents/MacOS/Arc",
];

fn pipe_to(mut command: Command, text: &str) -> bool {
   let Ok(mut child) = command
      .stdin(Stdio::piped())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn()
   else {
      return false;
   };
   let written = child
      .stdin
      .take()
      .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
   child.wait().is_ok_and(|status| status.success()) && written
}

pub struct Host;

impl DesktopHost for Host {
   fn copy(text: &str) -> Result<(), Error> {
      if pipe_to(Command::new("pbcopy"), text) {
         return Ok(());
      }
      Err(Error::Platform {
         message: "pbcopy failed".to_owned(),
      })
   }

   fn open(target: &str) -> Result<(), Error> {
      spawn_reaped(Command::new("open").arg(target))
   }

   fn app_browser() -> Option<Command> {
      APP_BROWSERS
         .into_iter()
         .map(PathBuf::from)
         .find(|path| path.is_file())
         .map(Command::new)
   }

   fn pick_js_path() -> Option<PathBuf> {
      let output = Command::new("osascript")
         .args([
            "-e",
            "POSIX path of (choose file of type {\"js\"} with prompt \"Install Sunbird Plugin (.js)\")",
         ])
         .stderr(Stdio::null())
         .output()
         .ok()?;
      let path = String::from_utf8(output.stdout).ok()?;
      let path = path.trim_end_matches('\n');
      (output.status.success() && !path.is_empty()).then(|| PathBuf::from(path))
   }

   fn notify(title: &str, body: &str) {
      let mut cmd = Command::new("osascript");
      cmd.args([
         "-e",
         "on run argv",
         "-e",
         "display notification (item 2 of argv) with title (item 1 of argv)",
         "-e",
         "end run",
         "--",
         title,
         body,
      ]);
      if let Err(err) = spawn_reaped(&mut cmd) {
         log::debug!("notify: {err}");
      }
   }
}
