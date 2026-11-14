//! Windows: embeds `assets/icon/icon.ico` as the exe icon (`icon.rc`).
//!
//! Voice (`livekit`) on macOS: WebRTC's Objective-C categories are only linked with `-ObjC`.
//! `webrtc-sys` asks for it, but a dependency's link arguments do not reach the final binary, and
//! without it WebRTC throws `+[NSString stringForStdString:]: unrecognized selector` at connect.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        println!("cargo:rerun-if-changed=icon.rc");
        embed_resource::compile("icon.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("icon.rc compiles");
    }
    let macos = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "macos");
    if macos && std::env::var_os("CARGO_FEATURE_VOICE").is_some() {
        println!("cargo:rustc-link-arg=-ObjC");
    }
}
