// The renderer's half of picture input: one file in, one bounded `data:` URL out.
//
// A phone photo is 4000 px and 8 MB, and none of that reaches the model: a vision projector resamples
// a picture onto its own fixed grid, so everything past ~1024 px on the long edge is bytes the app
// pays to transfer and the host charges tokens for while the model sees the same shapes. Shrinking
// here is the difference between a picture that answers in seconds and one that fills the context
// window before the question in it is read.
//
// Re-encoding to JPEG is the other half of the bound: a PNG of a screenshot can be several MB at 1024
// px, and the host reads the encoded URL, not the original file.

/** The longest edge a picture is kept at. The host charges each attached picture the engine's own
 * image-token cap, so this is the size at which that charge is honest rather than a guess. */
export const PICTURE_MAX_EDGE = 1024;

/** JPEG quality for the re-encode — high enough that a chart's axis labels survive, low enough that a
 * full-page screenshot stays well under the host's per-picture byte ceiling. */
const JPEG_QUALITY = 0.82;

/**
 * Shrink one picture to [`PICTURE_MAX_EDGE`] on its longest edge and return it as a
 * `data:image/jpeg;base64,…` URL. A file the browser cannot decode rejects, which is the honest
 * verdict: there is no picture to show the model, and saying so beats sending an empty attachment.
 */
export async function toPictureDataUrl(file: File): Promise<string> {
  const url = URL.createObjectURL(file);
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const scale = Math.min(1, PICTURE_MAX_EDGE / Math.max(image.naturalWidth, image.naturalHeight));
    const width = Math.max(1, Math.round(image.naturalWidth * scale));
    const height = Math.max(1, Math.round(image.naturalHeight * scale));
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("this browser offers no 2D canvas to shrink a picture with");
    // JPEG carries no alpha channel, so a transparent PNG's background becomes this rather than
    // black. It is the picture's matte, not the window's ink, which is why it is not a theme token.
    context.fillStyle = "white";
    context.fillRect(0, 0, width, height);
    context.drawImage(image, 0, 0, width, height);
    return canvas.toDataURL("image/jpeg", JPEG_QUALITY);
  } finally {
    URL.revokeObjectURL(url);
  }
}
