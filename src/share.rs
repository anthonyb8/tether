//! `share`, `unshare`, `shares` and `origin`: reaching a dev server from the
//! phone over the tailnet, or from anywhere with `share --public`.

use std::io::{self, IsTerminal};

use anyhow::Result;
use qrcode::QrCode;
use qrcode::render::unicode::Dense1x2;

use crate::tailscale;

pub fn share(port: u16, public: bool) -> Result<()> {
    let host = tailscale::host()?;
    let listen = if public {
        tailscale::funnel(port)?
    } else {
        tailscale::serve(port)?;
        port
    };
    let origin = tailscale::origin(&host, listen);
    println!("{origin}");
    // A QR code is for a person to scan; a script piping the output wants the
    // address alone.
    if io::stdout().is_terminal() {
        println!("{}", qr(&origin)?);
    }
    Ok(())
}

pub fn unshare(port: u16) -> Result<()> {
    tailscale::unserve(port)
}

pub fn shares() -> Result<()> {
    let shares = tailscale::shares()?;
    if shares.is_empty() {
        eprintln!("No ports are shared.");
    }
    for share in shares {
        let public = if share.public { " (public)" } else { "" };
        println!("{} -> {}{public}", share.origin, share.target);
    }
    Ok(())
}

pub fn origin(port: u16) -> Result<()> {
    println!("{}", tailscale::origin(&tailscale::host()?, port));
    Ok(())
}

fn qr(text: &str) -> Result<String> {
    let code = QrCode::new(text)?;
    // Terminals are mostly light text on dark, so a module drawn as a block
    // is light: draw the code's light modules as blocks to keep it the right
    // way round there.
    Ok(code
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .build())
}
