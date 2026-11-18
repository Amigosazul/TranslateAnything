//! Windows 进程工具：端口归属、进程路径与命令行、识别 Electron 应用。
//!
//! 沉浸模式靠它做"身份校验"：调试端口背后的进程必须就是配置里的那个程序，
//! 否则（比如别的程序恰好占了同一端口）一律不接管。

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
use windows::Win32::Foundation::{CloseHandle, HANDLE, UNICODE_STRING};
use windows::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER};
use windows::Win32::Networking::WinSock::AF_INET;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::Foundation::{FILETIME, HWND, LPARAM, WPARAM};
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    QueryFullProcessImageNameW, TerminateProcess,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE};
use windows::core::{BOOL, PWSTR};

/// 正在本机 127.0.0.1 / 0.0.0.0 上监听该 TCP 端口的进程 id
pub fn port_owner(port: u16) -> Option<u32> {
    unsafe {
        let mut size = 0u32;
        let _ = GetExtendedTcpTable(None, &mut size, false, AF_INET.0 as u32, TCP_TABLE_OWNER_PID_LISTENER, 0);
        if size == 0 {
            return None;
        }
        // 两次调用之间表可能变大，多留一些余量
        let mut buf = vec![0u8; size as usize + 4096];
        size = buf.len() as u32;
        if GetExtendedTcpTable(
            Some(buf.as_mut_ptr() as *mut c_void),
            &mut size,
            false,
            AF_INET.0 as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        ) != 0
        {
            return None;
        }
        let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let rows = std::slice::from_raw_parts(table.table.as_ptr(), table.dwNumEntries as usize);
        rows.iter()
            .find(|r| u16::from_be((r.dwLocalPort & 0xFFFF) as u16) == port)
            .map(|r| r.dwOwningPid)
    }
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn open(pid: u32) -> Option<Handle> {
    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok().map(Handle) }
}

/// 进程的完整程序路径（无权限读取的系统进程返回 None）
pub fn exe_path(pid: u32) -> Option<PathBuf> {
    let h = open(pid)?;
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    unsafe { QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).ok()? };
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

/// 进程的完整命令行
pub fn command_line(pid: u32) -> Option<String> {
    // ProcessCommandLineInformation：返回 UNICODE_STRING + 紧随其后的字符缓冲区（Windows 8.1 起）
    const PROCESS_COMMAND_LINE_INFORMATION: PROCESSINFOCLASS = PROCESSINFOCLASS(60);
    let h = open(pid)?;
    unsafe {
        let mut len = 0u32;
        let _ = NtQueryInformationProcess(h.0, PROCESS_COMMAND_LINE_INFORMATION, std::ptr::null_mut(), 0, &mut len);
        if len == 0 {
            return None;
        }
        // 用 u64 数组保证对齐
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let status = NtQueryInformationProcess(
            h.0,
            PROCESS_COMMAND_LINE_INFORMATION,
            buf.as_mut_ptr() as *mut c_void,
            len,
            &mut len,
        );
        if status.is_err() {
            return None;
        }
        let us = &*(buf.as_ptr() as *const UNICODE_STRING);
        if us.Buffer.is_null() {
            return None;
        }
        let chars = std::slice::from_raw_parts(us.Buffer.0, us.Length as usize / 2);
        Some(String::from_utf16_lossy(chars))
    }
}

/// 所有进程（pid, 进程名）
fn processes() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let snap = Handle(snap);
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap.0, &mut e).is_ok();
        while ok {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.push((e.th32ProcessID, String::from_utf16_lossy(&e.szExeFile[..n])));
            ok = Process32NextW(snap.0, &mut e).is_ok();
        }
    }
    out
}

/// 路径比较：不区分大小写、统一斜杠
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    norm(a) == norm(b)
}

/// 指定程序的所有进程 id
pub fn pids_of(exe: &Path) -> Vec<u32> {
    let Some(name) = exe.file_name().map(|n| n.to_string_lossy().to_lowercase()) else { return Vec::new() };
    processes()
        .into_iter()
        .filter(|(_, n)| n.to_lowercase() == name)
        .filter(|(pid, _)| exe_path(*pid).is_some_and(|p| same_path(&p, exe)))
        .map(|(pid, _)| pid)
        .collect()
}

/// Electron / Chromium 程序的主进程：命令行里不带 `--type=`（子进程都带 --type=renderer 等）
pub fn main_process(exe: &Path) -> Option<u32> {
    pids_of(exe)
        .into_iter()
        .find(|pid| command_line(*pid).is_some_and(|c| !c.contains("--type=")))
}

/// 进程已运行多久（读不到返回 None）
pub fn age(pid: u32) -> Option<std::time::Duration> {
    let h = open(pid)?;
    let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    let (mut created, mut a, mut b, mut c) = Default::default();
    unsafe {
        GetProcessTimes(h.0, &mut created, &mut a, &mut b, &mut c).ok()?;
        let now = GetSystemTimeAsFileTime();
        // FILETIME 单位是 100 纳秒
        Some(std::time::Duration::from_nanos(ft(now).saturating_sub(ft(created)) * 100))
    }
}

/// 请这些进程的顶层窗口关闭（等同点窗口的关闭按钮；带托盘的应用通常只是藏起来）
pub fn close_windows(pids: &[u32]) {
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
        let pids = unsafe { &*(lp.0 as *const Vec<u32>) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pids.contains(&pid) {
            let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        }
        true.into()
    }
    let list = pids.to_vec();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&list as *const Vec<u32> as isize));
    }
}

/// 强制结束这些进程
pub fn terminate(pids: &[u32]) {
    for &pid in pids {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let h = Handle(h);
                let _ = TerminateProcess(h.0, 1);
            }
        }
    }
}

/// 去掉命令行开头的程序路径（可能带引号）和已有的调试端口参数，只留参数部分
pub fn args_of(cmdline: &str) -> String {
    let c = cmdline.trim();
    let rest = if let Some(stripped) = c.strip_prefix('"') {
        stripped.find('"').map(|i| &stripped[i + 1..]).unwrap_or("")
    } else {
        c.find(' ').map(|i| &c[i + 1..]).unwrap_or("")
    };
    rest.split(' ')
        .filter(|a| !a.starts_with("--remote-debugging-port="))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// 程序目录看起来是 Electron 应用（带 asar 资源包和 Chromium 资源）
pub fn is_electron(exe: &Path) -> bool {
    let Some(dir) = exe.parent() else { return false };
    let res = dir.join("resources");
    (res.join("app.asar").exists() || res.join("app").join("package.json").exists())
        && (dir.join("resources.pak").exists() || dir.join("chrome_100_percent.pak").exists())
}

/// 正在运行的 Electron 应用：程序路径 → 是否已开着调试端口（端口号）
pub fn running_electron_apps() -> BTreeMap<PathBuf, Option<u16>> {
    let me = std::env::current_exe().ok();
    let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();
    for (pid, name) in processes() {
        if !name.to_lowercase().ends_with(".exe") {
            continue;
        }
        if let Some(p) = exe_path(pid) {
            seen.entry(p.to_string_lossy().to_lowercase()).or_insert(p);
        }
    }
    let mut out = BTreeMap::new();
    for p in seen.into_values() {
        if me.as_ref().is_some_and(|m| same_path(m, &p)) || !is_electron(&p) {
            continue;
        }
        let port = main_process(&p)
            .and_then(command_line)
            .and_then(|c| {
                c.split(' ')
                    .find_map(|a| a.strip_prefix("--remote-debugging-port=").map(str::to_string))
            })
            .and_then(|s| s.trim_matches('"').parse().ok());
        out.insert(p, port);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 拆出启动参数() {
        assert_eq!(args_of(r#""C:\A B\app.exe" --x=1 --remote-debugging-port=9 --y"#), "--x=1 --y");
        assert_eq!(args_of(r"C:\app.exe --lang=en-US"), "--lang=en-US");
        assert_eq!(args_of(r"C:\app.exe"), "");
    }

    #[test]
    fn 读取本进程信息() {
        let pid = std::process::id();
        let me = std::env::current_exe().unwrap();
        assert!(same_path(&exe_path(pid).unwrap(), &me));
        assert!(command_line(pid).unwrap().len() > 3);
        assert!(pids_of(&me).contains(&pid));
    }

    #[test]
    fn 运行时长与强制结束() {
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(age(pid).unwrap() < std::time::Duration::from_secs(10));
        terminate(&[pid]);
        assert_eq!(child.wait().unwrap().code(), Some(1));
    }

    #[test]
    fn 端口归属() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert_eq!(port_owner(port), Some(std::process::id()));
    }
}
