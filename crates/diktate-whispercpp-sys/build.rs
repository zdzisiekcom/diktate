//! Budowanie whisper.cpp i warstwy pośredniej z `src/shim.cpp`.
//!
//! Źródła bierzemy z submodułu `vendor/whisper.cpp` przypiętego do wydania.
//! `WHISPER_CPP_DIR` nadpisuje ten wybór i wskazuje własny klon — przydaje
//! się, gdy ktoś ma już zbudowane drzewo i nie chce kompilować go po raz
//! drugi. Ta sama zasada, co przy `DIKTATE_MODEL`: zmienna środowiskowa
//! nadpisuje wybór domyślny, ale repozytorium zostaje samowystarczalne.
//!
//! Biblioteki linkujemy statycznie, więc daemon jest jednym plikiem bez
//! `rpath` i bez `.so` obok. Kolejność ma znaczenie: shim woła whispera
//! i parakeeta, oni wołają ggml — linker GNU wymaga, żeby zależny szedł
//! przed zależnością.

use std::path::{Path, PathBuf};

fn main() {
    let source = source_dir();
    println!("cargo:rerun-if-env-changed=WHISPER_CPP_DIR");
    println!("cargo:rerun-if-changed=src/shim.cpp");
    println!("cargo:rerun-if-changed=src/shim.h");

    let mut config = cmake::Config::new(&source);
    config
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("WHISPER_BUILD_TESTS", "OFF")
        .define("WHISPER_BUILD_EXAMPLES", "OFF")
        .define("WHISPER_ALL_WARNINGS", "OFF")
        .define("WHISPER_ALL_WARNINGS_3RD_PARTY", "OFF")
        .define("GGML_METAL", "OFF")
        // Biblioteki statyczne trafiają do rustowej binarki, która jest
        // budowana jako PIE — bez tego linker odmawia.
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        // Debugowy build ggml jest zbyt wolny, żeby dyktować: whisper-rs-sys
        // z tego samego powodu wymusza optymalizacje niezależnie od profilu.
        .define("CMAKE_BUILD_TYPE", "Release");

    if cfg!(feature = "vulkan") {
        config.define("GGML_VULKAN", "ON");
    }

    let destination = config.build();

    // Warstwa pośrednia. Kompilowana jako pierwsza, bo `cc` wypisuje swoją
    // dyrektywę linkowania od razu, a shim musi trafić przed whisperem.
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("src/shim.cpp")
        .include("src")
        .include(source.join("include"))
        .include(source.join("ggml/include"))
        .compile("diktate_shim");

    // Statyczne archiwa zostają w drzewie budowania: reguła `install`
    // w whisper.cpp obejmuje `LIBRARY`, czyli biblioteki współdzielone,
    // a nie `ARCHIVE`. Stąd przeszukiwanie katalogu budowania, a nie tylko
    // prefiksu instalacji.
    println!("cargo:rustc-link-search=native={}", destination.display());
    for dir in ["lib", "lib64", "build"] {
        add_search_paths(&destination.join(dir));
    }

    println!("cargo:rustc-link-lib=static=whisper");
    println!("cargo:rustc-link-lib=static=parakeet");
    println!("cargo:rustc-link-lib=static=ggml");
    println!("cargo:rustc-link-lib=static=ggml-base");
    println!("cargo:rustc-link-lib=static=ggml-cpu");
    if cfg!(feature = "vulkan") {
        println!("cargo:rustc-link-lib=static=ggml-vulkan");
        println!("cargo:rustc-link-lib=dylib=vulkan");
    }
    // ggml domyślnie korzysta z OpenMP, a C++ ze swojej biblioteki
    // standardowej — obie są dynamiczne i muszą być wskazane jawnie.
    println!("cargo:rustc-link-lib=dylib=gomp");
    println!("cargo:rustc-link-lib=dylib=stdc++");
}

/// Katalog ze źródłami whisper.cpp: `WHISPER_CPP_DIR` albo submoduł.
///
/// Brak `src/parakeet.cpp` znaczy prawie zawsze jedno — repozytorium
/// sklonowano bez submodułów. Mówimy to wprost, bo błąd cmake o brakującym
/// `CMakeLists.txt` niczego nie tłumaczy.
fn source_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("WHISPER_CPP_DIR") {
        let dir = PathBuf::from(dir);
        assert!(
            dir.join("src/parakeet.cpp").is_file(),
            "WHISPER_CPP_DIR wskazuje {}, ale nie ma tam src/parakeet.cpp — \
             to nie jest klon whisper.cpp albo jest starszy niż 1.9.4",
            dir.display()
        );
        return dir;
    }
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendored = manifest
        .parent()
        .and_then(Path::parent)
        .expect("crate leży w crates/<nazwa>")
        .join("vendor/whisper.cpp");
    assert!(
        vendored.join("src/parakeet.cpp").is_file(),
        "brak źródeł whisper.cpp w {} — uruchom `git submodule update --init` \
         albo wskaż własny klon zmienną WHISPER_CPP_DIR",
        vendored.display()
    );
    vendored
}

/// Dokłada katalog i wszystkie jego podkatalogi do ścieżek przeszukiwania.
/// Archiwa ggml leżą rozrzucone po drzewie budowania (`ggml/src`,
/// `ggml/src/ggml-vulkan`), więc jeden katalog nie wystarcza.
fn add_search_paths(dir: &Path) {
    if !dir.is_dir() {
        return;
    }
    println!("cargo:rustc-link-search=native={}", dir.display());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            add_search_paths(&entry.path());
        }
    }
}
