fn main() {
    // Writes target/aarch64-linux-android/<profile>/gradle/companion/, the
    // whole Gradle project that scripts/android.sh packages.
    //
    // Once libmorph releases URL schemes (its plan
    // 20261022_07_URL_SCHEMES_PLAN.md, Draft), the `handlr://` intent
    // filter goes here as `.url_scheme("handlr")`; libmorph-build 0.2.1 has
    // no such call.
    libmorph_build::gradle_project("dev.tmzt.handlr")
        .label("handlr")
        .lib_name("companion")
        // libmorph asks for it after init once the manifest declares it.
        .permission("android.permission.RECORD_AUDIO")
        // The link to handlr, over Tailscale or the LAN.
        .permission("android.permission.INTERNET")
        .generate()
        .expect("libmorph-build: generating the Gradle project");
}
