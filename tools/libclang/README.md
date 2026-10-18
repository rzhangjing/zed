# libclang (vendored, Windows x64)

`libclang.dll` is here so that `cargo check --workspace --all-targets` works on a
machine without LLVM installed: `llama-cpp-sys-2` runs `bindgen`, and `bindgen`
`dlopen()`s this DLL.

Cargo points `LIBCLANG_PATH` at this directory from the repository's
`.cargo/config.toml`:

```toml
[env]
LIBCLANG_PATH = { value = "tools/libclang", relative = true }
```

Nothing else is needed: no `LIBCLANG_PATH` in the environment, no user-level
`~/.cargo/config.toml`, no LLVM install.

| | |
| --- | --- |
| Version | libclang 18.1.1 |
| Origin | `libclang-18.1.1-py2.py3-none-win_amd64.whl` from PyPI (project: <https://github.com/sighingnow/libclang>, mirroring <https://github.com/llvm/llvm-project/tree/main/clang/bindings/python>) |
| SHA-256 | `B13EC96D9A036D6C6C36DD09E00415EAD7ACC7FCFD40E721BF33EA86B3490F9D` |
| Size | 80.1 MiB |
| License | Apache-2.0 WITH LLVM-exception, see `LICENSE.TXT` |
| Platform | Windows x86_64 only — on macOS/Linux use the system LLVM instead |

## Refreshing

```powershell
python -m pip download libclang --no-deps -d "$env:TEMP\libclang-dl"
tar -xf "$env:TEMP\libclang-dl\libclang-*.whl" -C "$env:TEMP\libclang-dl"
Copy-Item "$env:TEMP\libclang-dl\libclang-*.data\platlib\clang\native\libclang.dll" tools\libclang\
Copy-Item "$env:TEMP\libclang-dl\libclang-*.dist-info\LICENSE.TXT" tools\libclang\
```

Then update the version and checksum in this file.

## Other platforms

Drop the `LIBCLANG_PATH` entry in `.cargo/config.toml` (or point it at your LLVM
`bin` directory) and install LLVM normally — on macOS
`brew install llvm`, on Debian/Ubuntu `apt install libclang-dev`, on Arch
`pacman -S clang`. `bindgen` also honours `LIBCLANG_PATH`, so setting the
environment variable to any directory holding `libclang.so`/`libclang.dylib`
works without touching this file.
