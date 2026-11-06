//! Права и владельцы на Windows для канала агента (#160).
//!
//! Файл подключается и приложением (`lib.rs`), и посредником `darwaza-mcp`
//! (`#[path]`): посредник не линкуется с библиотекой приложения, а логика
//! проверки должна быть одна — две копии расходятся молча.
//!
//! Что здесь решается. Имена именованных каналов видит любой пользователь
//! машины, а создать канал с любым именем может кто угодно. Поэтому:
//! - приложение создаёт канал с явным списком доступа — только текущему
//!   пользователю (`OwnerOnly`), а не с умолчанием системы;
//! - посредник перед отправкой токена убеждается, что канал создан процессом
//!   текущего пользователя (`server_is_current_user`): занять имя заранее
//!   чужой процесс может, но получить токен — нет.
//! - файл токена закрывается тем же способом (`restrict_to_current_user`).

#![allow(dead_code)] // посредник подключает файл целиком, а использует часть

use std::ffi::c_void;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetSecurityDescriptorDacl, GetTokenInformation, TokenUser,
    DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// Дескриптор, который закрывается сам.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn last_error(what: &str) -> io::Error {
    let e = io::Error::last_os_error();
    io::Error::new(e.kind(), format!("{what}: {e}"))
}

/// Пользователь токена: буфер с `TOKEN_USER`, SID указывает внутрь него.
struct TokenUserInfo(Vec<u8>);

impl TokenUserInfo {
    fn of_token(token: HANDLE) -> io::Result<Self> {
        let mut len = 0u32;
        // Первый вызов нужен только за размером: он обязан упасть с
        // ERROR_INSUFFICIENT_BUFFER и оставить длину в `len`.
        unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len) };
        if len == 0 {
            return Err(last_error("размер данных о пользователе токена"));
        }
        let mut buf = vec![0u8; len as usize];
        let ok = unsafe {
            GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast::<c_void>(), len, &mut len)
        };
        if ok == 0 {
            return Err(last_error("данные о пользователе токена"));
        }
        Ok(Self(buf))
    }

    /// SID внутри буфера. Живёт, пока жив `self`.
    fn sid(&self) -> PSID {
        // Буфер выровнен по байту, а не по восьми: читаем без выравнивания.
        let user = unsafe { std::ptr::read_unaligned(self.0.as_ptr().cast::<TOKEN_USER>()) };
        user.User.Sid
    }
}

fn current_user() -> io::Result<TokenUserInfo> {
    let mut token: HANDLE = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error("токен текущего процесса"));
    }
    let token = Handle(token);
    TokenUserInfo::of_token(token.0)
}

/// SID текущего пользователя в строковом виде (`S-1-5-21-…`).
pub fn current_user_sid_string() -> io::Result<String> {
    let user = current_user()?;
    let mut wide: *mut u16 = null_mut();
    if unsafe { ConvertSidToStringSidW(user.sid(), &mut wide) } == 0 {
        return Err(last_error("SID пользователя в текст"));
    }
    let mut len = 0usize;
    while unsafe { *wide.add(len) } != 0 {
        len += 1;
    }
    let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(wide, len) });
    unsafe { LocalFree(wide.cast::<c_void>()) };
    Ok(text)
}

/// Дескриптор безопасности «полный доступ только текущему пользователю, без
/// наследования»: `D:P(A;;GA;;;<SID>)`.
struct OwnerOnlyDescriptor(PSECURITY_DESCRIPTOR);

impl OwnerOnlyDescriptor {
    fn new() -> io::Result<Self> {
        let sddl = format!("D:P(A;;GA;;;{})", current_user_sid_string()?);
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        };
        if ok == 0 {
            return Err(last_error("список доступа только для пользователя"));
        }
        Ok(Self(sd))
    }
}

impl Drop for OwnerOnlyDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0.cast::<c_void>()) };
        }
    }
}

/// Создать экземпляр канала, доступный только текущему пользователю.
///
/// Всё синхронно и внутри одной функции: сырые указатели не должны жить через
/// `.await`, иначе будущее перестаёт быть `Send`. Не получилось построить
/// список доступа — это ошибка, а не повод создать канал с умолчаниями.
pub fn create_owner_only_pipe(
    name: &str,
    first_instance: bool,
) -> io::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    let descriptor = OwnerOnlyDescriptor::new()?;
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let mut options = tokio::net::windows::named_pipe::ServerOptions::new();
    options
        .first_pipe_instance(first_instance)
        .reject_remote_clients(true);
    // SAFETY: `attributes` и дескриптор живут до конца вызова; система копирует
    // их при создании канала.
    unsafe {
        options.create_with_security_attributes_raw(
            name,
            (&mut attributes as *mut SECURITY_ATTRIBUTES).cast::<c_void>(),
        )
    }
}

/// Создан ли канал процессом текущего пользователя.
///
/// Посредник зовёт это ДО отправки токена. `false` или ошибка — не доверять:
/// канал с этим именем мог занять посторонний.
pub fn server_is_current_user(pipe: &impl AsRawHandle) -> io::Result<bool> {
    let mut pid = 0u32;
    if unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle() as HANDLE, &mut pid) } == 0 {
        return Err(last_error("процесс-владелец канала"));
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(last_error("доступ к процессу-владельцу канала"));
    }
    let process = Handle(process);
    let mut token: HANDLE = null_mut();
    if unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error("токен процесса-владельца канала"));
    }
    let token = Handle(token);
    let theirs = TokenUserInfo::of_token(token.0)?;
    let ours = current_user()?;
    Ok(unsafe { EqualSid(theirs.sid(), ours.sid()) } != 0)
}

/// Закрыть файл от всех, кроме текущего пользователя: свой список доступа без
/// наследования от каталога (Администраторы и прочие из каталога не остаются).
///
/// Зовётся на ПУСТОМ файле, до записи содержимого: сначала замок, потом секрет.
pub fn restrict_to_current_user(path: &std::path::Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    let descriptor = OwnerOnlyDescriptor::new()?;
    let mut present = 0i32;
    let mut dacl = null_mut();
    let mut defaulted = 0i32;
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) }
        == 0
        || present == 0
    {
        return Err(last_error("список доступа из дескриптора"));
    }
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let code = unsafe {
        SetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            dacl,
            null(),
        )
    };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    Ok(())
}
