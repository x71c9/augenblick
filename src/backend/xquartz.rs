// macOS has no X server of its own: XQuartz provides it. It is not part of
// the system since OS X 10.8 and is installed separately, so the errors
// below say whether it is missing or only not running.

use std::path::Path;

const APP: &str = "/Applications/Utilities/XQuartz.app";

fn installed() -> bool {
  Path::new(APP).exists()
}

// DISPLAY is not set
pub fn no_display() -> String {
  if installed() {
    "DISPLAY is not set. XQuartz sets it for the sessions started after a \
     logout and login; until then, start XQuartz with `open -a XQuartz` and \
     run `DISPLAY=:0 augenblick`"
      .to_string()
  } else {
    not_installed()
  }
}

// DISPLAY is set, but no X server answers on it
pub fn no_connection(display: &str, error: &dyn std::fmt::Display) -> String {
  if installed() {
    format!(
      "cannot connect to the X server at DISPLAY={display} ({error}). \
       XQuartz is not running: start it with `open -a XQuartz`"
    )
  } else {
    not_installed()
  }
}

fn not_installed() -> String {
  "augenblick needs XQuartz on macOS, and XQuartz is not installed. \
   Install it with `brew install --cask xquartz` \
   (or from https://www.xquartz.org), start it with `open -a XQuartz`, \
   then run `DISPLAY=:0 augenblick`"
    .to_string()
}
