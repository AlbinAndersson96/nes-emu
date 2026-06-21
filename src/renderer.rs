use pixels::{Pixels, SurfaceTexture};
use winit::{
    dpi::LogicalSize,
    event_loop::EventLoop,
    window::{Window, WindowBuilder, WindowId},
};

pub const NES_PALETTE: [(u8, u8, u8); 64] = [
    (0x54, 0x54, 0x54), (0x00, 0x1E, 0x74), (0x08, 0x10, 0x90), (0x30, 0x00, 0x88),
    (0x44, 0x00, 0x64), (0x5C, 0x00, 0x30), (0x54, 0x04, 0x00), (0x3C, 0x18, 0x00),
    (0x20, 0x2A, 0x00), (0x08, 0x3A, 0x00), (0x00, 0x40, 0x00), (0x00, 0x3C, 0x00),
    (0x00, 0x32, 0x3C), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
    (0x98, 0x96, 0x98), (0x08, 0x4C, 0xC4), (0x30, 0x32, 0xEC), (0x5C, 0x1E, 0xE4),
    (0x88, 0x14, 0xB0), (0xA0, 0x14, 0x64), (0x98, 0x22, 0x20), (0x78, 0x3C, 0x00),
    (0x54, 0x5A, 0x00), (0x28, 0x72, 0x00), (0x08, 0x7C, 0x00), (0x00, 0x76, 0x28),
    (0x00, 0x66, 0x78), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
    (0xEC, 0xEE, 0xEC), (0x4C, 0x9A, 0xEC), (0x78, 0x7C, 0xEC), (0xB0, 0x62, 0xEC),
    (0xE4, 0x54, 0xEC), (0xEC, 0x58, 0xA4), (0xEC, 0x6A, 0x64), (0xD4, 0x88, 0x20),
    (0xA0, 0xAA, 0x00), (0x74, 0xC4, 0x00), (0x4C, 0xD0, 0x20), (0x38, 0xCC, 0x6C),
    (0x38, 0xB4, 0xCC), (0x3C, 0x3C, 0x3C), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
    (0xEC, 0xEE, 0xEC), (0xA8, 0xCC, 0xEC), (0xBC, 0xBC, 0xEC), (0xD4, 0xB2, 0xEC),
    (0xEC, 0xAE, 0xEC), (0xEC, 0xAE, 0xD4), (0xEC, 0xB4, 0xB0), (0xE4, 0xC4, 0x90),
    (0xCC, 0xD2, 0x78), (0xB4, 0xDE, 0x78), (0xA8, 0xE2, 0x90), (0x98, 0xE2, 0xB4),
    (0xA0, 0xD6, 0xE4), (0xA0, 0xA2, 0xA0), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
];

pub(crate) fn nes_to_rgba(frame: &[u8; 256 * 240], buf: &mut [u8]) {
    for (i, &idx) in frame.iter().enumerate() {
        let (r, g, b) = NES_PALETTE[(idx & 0x3F) as usize];
        buf[i * 4]     = r;
        buf[i * 4 + 1] = g;
        buf[i * 4 + 2] = b;
        buf[i * 4 + 3] = 255;
    }
}

pub struct Renderer {
    window: Window,
    pixels: Pixels,
}

impl Renderer {
    pub fn new(event_loop: &EventLoop<()>) -> Result<Self, Box<dyn std::error::Error>> {
        let window = WindowBuilder::new()
            .with_title("nes-emu")
            .with_inner_size(LogicalSize::new(512u32, 480u32))
            .build(event_loop)?;
        let size = window.inner_size();
        let surface_texture = SurfaceTexture::new(size.width, size.height, &window);
        let pixels = Pixels::new(256, 240, surface_texture)?;
        Ok(Self { window, pixels })
    }

    pub fn present(&mut self, frame: &[u8; 256 * 240]) -> Result<(), pixels::Error> {
        nes_to_rgba(frame, self.pixels.frame_mut());
        self.pixels.render()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let _ = self.pixels.resize_surface(width, height);
    }

    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_index_0_maps_correctly() {
        let frame = [0u8; 256 * 240];
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        let (r, g, b) = NES_PALETTE[0];
        assert_eq!(&buf[0..4], &[r, g, b, 255]);
    }

    #[test]
    fn palette_index_masked_to_6_bits() {
        let mut frame = [0u8; 256 * 240];
        frame[0] = 0x41; // 0x41 & 0x3F == 0x01
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        let (r, g, b) = NES_PALETTE[0x01];
        assert_eq!(&buf[0..4], &[r, g, b, 255]);
    }

    #[test]
    fn alpha_channel_always_255() {
        let frame = [0x3Fu8; 256 * 240];
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        for chunk in buf.chunks(4) {
            assert_eq!(chunk[3], 255, "alpha must be 255 for all pixels");
        }
    }

    #[test]
    fn last_pixel_is_written() {
        let mut frame = [0u8; 256 * 240];
        let last = 256 * 240 - 1;
        frame[last] = 0x30;
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        let (r, g, b) = NES_PALETTE[0x30];
        assert_eq!(&buf[last * 4..last * 4 + 4], &[r, g, b, 255]);
    }
}
