use core::sync::atomic::AtomicUsize;

use crate::rast::Pixel;

extern "C" {
    #[allow(improper_ctypes)]
    fn unpack_1bpp_impl(
        input_line: *const u32,
        clut: *const AtomicUsize,
        render_target: *mut Pixel,
        words_in_input: usize,
    );
}

/// Rasterize packed 1bpp pixel data using a color lookup table (CLUT).
///
/// `src` is a scanline of pixel data packed into `u32`s, where the least
/// significant bit of each `u32` is on the left.
///
/// `clut` is a reference to the CLUT, stored in the low two bytes of an
/// `AtomicUsize`. The least significant byte is the color used for 0 bits, and
/// the next byte for 1 bits. The top two bytes are unused.
///
/// `target` is the destination for unpacked raster output.
///
/// `src.len()` should be exactly `target.len() / 32`. Otherwise the results are
/// safe but undefined.
pub fn unpack(src: &[u32], clut: &AtomicUsize, target: &mut [u8]) {
    assert_eq!(src.len() * 32, target.len());
    // Safety: the assembler routine is safe as long as the assertion above
    // holds.
    unsafe {
        unpack_1bpp_impl(src.as_ptr(), clut, target.as_mut_ptr(), src.len())
    }
}
