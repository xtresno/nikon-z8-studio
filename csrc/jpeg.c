/* JPEG decode and encode through the system libjpeg-turbo, the same library
 * family Pillow uses, so draft decoding, quality and subsampling mean exactly
 * what they meant in the Python app. Every call has its own codec struct:
 * nothing here is shared between threads. */
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <jpeglib.h>

struct err {
    struct jpeg_error_mgr pub;
    jmp_buf jb;
    char *msg;
};

static void on_error(j_common_ptr c) {
    struct err *e = (struct err *)c->err;
    if (e->msg) (*c->err->format_message)(c, e->msg);
    longjmp(e->jb, 1);
}

static void quiet(j_common_ptr c, int level) { (void)c; (void)level; }

typedef unsigned char *(*alloc_fn)(void *ctx, size_t n);

/* Header only: stored size and component count. */
int z8j_info(const unsigned char *buf, unsigned long len, int *w, int *h,
             int *comps, char *msg) {
    struct jpeg_decompress_struct d;
    struct err e;
    d.err = jpeg_std_error(&e.pub);
    e.pub.error_exit = on_error;
    e.pub.emit_message = quiet;
    e.msg = msg;
    if (setjmp(e.jb)) { jpeg_destroy_decompress(&d); return 1; }
    jpeg_create_decompress(&d);
    jpeg_mem_src(&d, buf, len);
    jpeg_read_header(&d, TRUE);
    *w = d.image_width; *h = d.image_height; *comps = d.num_components;
    jpeg_destroy_decompress(&d);
    return 0;
}

/* Decode at 1/denom scale (1, 2, 4 or 8) to packed RGB, into memory the
 * caller allocates once the output size is known. */
int z8j_decode(const unsigned char *buf, unsigned long len, int denom,
               alloc_fn alloc, void *ctx, int *w, int *h, char *msg) {
    struct jpeg_decompress_struct d;
    struct err e;
    d.err = jpeg_std_error(&e.pub);
    e.pub.error_exit = on_error;
    e.pub.emit_message = quiet;
    e.msg = msg;
    if (setjmp(e.jb)) { jpeg_destroy_decompress(&d); return 1; }
    jpeg_create_decompress(&d);
    jpeg_mem_src(&d, buf, len);
    jpeg_read_header(&d, TRUE);
    d.out_color_space = JCS_RGB;
    d.scale_num = 1;
    d.scale_denom = denom > 0 ? denom : 1;
    jpeg_start_decompress(&d);
    *w = d.output_width; *h = d.output_height;
    size_t stride = (size_t)d.output_width * 3;
    unsigned char *out = alloc(ctx, stride * d.output_height);
    if (!out) { jpeg_destroy_decompress(&d); return 2; }
    while (d.output_scanline < d.output_height) {
        JSAMPROW row = out + stride * d.output_scanline;
        jpeg_read_scanlines(&d, &row, 1);
    }
    jpeg_finish_decompress(&d);
    jpeg_destroy_decompress(&d);
    return 0;
}

/* Encode packed RGB (comps 3) or gray (comps 1). subsampling: 0 = 4:4:4,
 * 2 = 4:2:0, -1 = the library default, as Pillow names them. The buffer is
 * malloc'd by libjpeg; free it with z8j_free. */
int z8j_encode(const unsigned char *px, int w, int h, int comps, int quality,
               int subsampling, int optimize, int dpi, int progressive,
               unsigned char **out, unsigned long *outlen, char *msg) {
    struct jpeg_compress_struct c;
    struct err e;
    c.err = jpeg_std_error(&e.pub);
    e.pub.error_exit = on_error;
    e.pub.emit_message = quiet;
    e.msg = msg;
    *out = NULL; *outlen = 0;
    if (setjmp(e.jb)) {
        jpeg_destroy_compress(&c);
        if (*out) { free(*out); *out = NULL; }
        return 1;
    }
    jpeg_create_compress(&c);
    jpeg_mem_dest(&c, out, outlen);
    c.image_width = w; c.image_height = h;
    c.input_components = comps;
    c.in_color_space = comps == 1 ? JCS_GRAYSCALE : JCS_RGB;
    jpeg_set_defaults(&c);
    jpeg_set_quality(&c, quality, TRUE);
    if (comps == 3 && subsampling == 0) {
        c.comp_info[0].h_samp_factor = 1; c.comp_info[0].v_samp_factor = 1;
    } else if (comps == 3 && subsampling == 1) {
        c.comp_info[0].h_samp_factor = 2; c.comp_info[0].v_samp_factor = 1;
    } else if (comps == 3 && subsampling == 2) {
        c.comp_info[0].h_samp_factor = 2; c.comp_info[0].v_samp_factor = 2;
    }
    if (optimize) c.optimize_coding = TRUE;
    if (progressive) jpeg_simple_progression(&c);
    if (dpi > 0) {
        c.write_JFIF_header = TRUE;
        c.density_unit = 1;
        c.X_density = dpi; c.Y_density = dpi;
    }
    jpeg_start_compress(&c, TRUE);
    size_t stride = (size_t)w * comps;
    while (c.next_scanline < c.image_height) {
        JSAMPROW row = (JSAMPROW)(px + stride * c.next_scanline);
        jpeg_write_scanlines(&c, &row, 1);
    }
    jpeg_finish_compress(&c);
    jpeg_destroy_compress(&c);
    return 0;
}

void z8j_free(unsigned char *p) { free(p); }
