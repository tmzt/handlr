fn main() {
    // Writes target/aarch64-linux-android/<profile>/gradle/companion/, the
    // whole Gradle project that scripts/android.sh packages.
    libmorph_build::gradle_project("dev.tmzt.handlr")
        .label("handlr")
        // `handlr://<host>:<port>/pair#<pairing string>` links open the app
        // and pair it (libmorph-build 0.2.2).
        .url_scheme("handlr")
        .lib_name("companion")
        // libmorph asks for it after init once the manifest declares it.
        .permission("android.permission.RECORD_AUDIO")
        // The link to handlr, over Tailscale or the LAN.
        .permission("android.permission.INTERNET")
        .generate()
        .expect("libmorph-build: generating the Gradle project");
}
