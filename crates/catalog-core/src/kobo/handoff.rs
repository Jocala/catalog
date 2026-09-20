//! Handoff — stacking fix without KOReader source (black-box OCP).
//!
//! Generation is pure; I/O is via `super::kobo::ssh_sync` (ssh -T heredoc).
//! All shell fragments are `sh` compatible with busybox on Kobo.

pub const HANDOFF_SHELL_GUARD: &str = concat!(
    "if pidof reader.lua >/dev/null 2>&1; then\n",
    "  if [ -n \"${1}\" ]; then\n",
    "    printf '%s\\n' \"${1}\" >\"/tmp/koreader-handoff.tmp.$$\"\n",
    "    mv -f \"/tmp/koreader-handoff.tmp.$$\" \"/tmp/koreader-handoff\"\n",
    "  fi\n",
    "  exit 0\n",
    "fi\n"
);

pub const HANDOFF_LUA_PATCH: &str = r#"-- 2-handoff.lua — stacking fix drop-in, no source rebuild.
local lfs=require("libs/libkoreader-lfs")
local logger=require("logger")
local Handoff={path="/tmp/koreader-handoff",interval=1.0,armed=false}
function Handoff:init() if self.armed then return end; self.armed=true; require("ui/uimanager"):scheduleIn(self.interval,function() self:tick() end) end
function Handoff:tick() if self.armed then require("ui/uimanager"):scheduleIn(self.interval,function() self:tick() end) end; local ok,err=pcall(function() self:check() end); if not ok then logger.warn("Handoff:",err) end end
function Handoff:check()
  if lfs.attributes(self.path,"mode")~="file" then return end
  local f=io.open(self.path,"r"); if not f then return end; local t=f:read("*l"); f:close()
  if not t or t=="" then os.remove(self.path); return end
  if lfs.attributes(t,"mode")~="file" then logger.warn("Handoff target missing:",t); os.remove(self.path); return end
  local ok,ReaderUI=pcall(require,"apps/reader/readerui")
  if ok and ReaderUI.instance then logger.info("Handoff: switching to",t); os.remove(self.path); ReaderUI.instance:switchDocument(t); return end
  local ok2,FM=pcall(require,"apps/filemanager/filemanager")
  if ok2 and FM.instance then local fu=require("apps/filemanager/filemanagerutil"); logger.info("Handoff from filemanager",t); os.remove(self.path); fu.openFile(FM.instance,t); return end
end
function Handoff:quit() self.armed=false end
pcall(function() Handoff:init() end)
"#;

pub fn is_handoff_check_cmd() -> String {
    "grep -q koreader-handoff /mnt/onboard/.adds/koreader/koreader.sh 2>/dev/null && test -f /mnt/onboard/.adds/koreader/patches/2-handoff.lua && echo ok || echo missing".to_string()
}

pub fn is_koreader_present_check_cmd() -> String {
    "test -d /mnt/onboard/.adds/koreader && echo present || echo absent".to_string()
}

pub fn install_cmds() -> Vec<String> {
    vec![
        "mkdir -p /mnt/onboard/.adds/koreader/patches".to_string(),
        // write lua patch atomically
        format!(
            "cat > /tmp/2-handoff.lua.tmp <<'KOBO_HO_EOF'\n{}\nKOBO_HO_EOF\nmv -f /tmp/2-handoff.lua.tmp /mnt/onboard/.adds/koreader/patches/2-handoff.lua",
            HANDOFF_LUA_PATCH
        ),
        // patch koreader.sh if not already patched: backup once, then insert guard after relocalize
        r#"if ! grep -q koreader-handoff /mnt/onboard/.adds/koreader/koreader.sh; then cp -p /mnt/onboard/.adds/koreader/koreader.sh /mnt/onboard/.adds/koreader/koreader.sh.orig-handoff 2>/dev/null; awk 'NR==1,0 {print} /exec "\/tmp\/koreader\.sh"/ {print ""; print "if pidof reader.lua >/dev/null 2>&1; then"; print "  if [ -n \"${1}\" ]; then"; print "    printf '\''%s\\n'\'' \"${1}\" >\"/tmp/koreader-handoff.tmp.$$\""; print "    mv -f \"/tmp/koreader-handoff.tmp.$$\" \"/tmp/koreader-handoff\""; print "  fi"; print "  exit 0"; print "fi"} 1' /mnt/onboard/.adds/koreader/koreader.sh > /tmp/koreader.sh.new && mv /tmp/koreader.sh.new /mnt/onboard/.adds/koreader/koreader.sh && chmod +x /mnt/onboard/.adds/koreader/koreader.sh; fi; sync"#.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guard_contains_handoff() {
        assert!(HANDOFF_SHELL_GUARD.contains("koreader-handoff"));
        assert!(HANDOFF_SHELL_GUARD.contains("pidof reader.lua"));
    }
    #[test]
    fn lua_patch_contains_switch() {
        assert!(HANDOFF_LUA_PATCH.contains("switchDocument"));
        assert!(HANDOFF_LUA_PATCH.contains("/tmp/koreader-handoff"));
    }
    #[test]
    fn check_cmd_shape() {
        let c = is_handoff_check_cmd();
        assert!(c.contains("grep -q koreader-handoff"));
        assert!(c.contains("2-handoff.lua"));
    }
}
