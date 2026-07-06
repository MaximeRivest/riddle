// image_demo: render an image file through quill takeover.
// Usage: image_demo /path/to/image.png [fit|fill|stretch] [ordered|fs|threshold|gray]
// Renders once, then waits. Exit: power button, 5-finger tap, or SIGTERM.
//
// `ordered` is the default: pure black/white with Bayer dot-density dithering,
// which looks map-like and avoids unstable gray waveforms.

#include <QImage>
#include <QtGlobal>
#include <stdio.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <signal.h>
#include <stdint.h>
#include <sys/ioctl.h>
#include <sys/time.h>
#include <poll.h>
#include <vector>

extern "C" {
int quill_init(void);
int quill_width(void);
int quill_height(void);
int quill_stride(void);
int quill_format(void);
unsigned char *quill_buffer(void);
unsigned long quill_swap(int x, int y, int w, int h, int mode, int full);
void quill_process_events(void);
}

#define EV_KEY 1
#define EV_ABS 3
#define ABS_MT_SLOT 47
#define ABS_MT_TRACKING_ID 57
#define KEY_POWER 116
#define EVIOCGRAB 0x40044590
#define MAX_SLOTS 16

struct input_event {
    struct timeval time;
    uint16_t type;
    uint16_t code;
    int32_t value;
};

static volatile sig_atomic_t g_quit = 0;
static void on_term(int sig) { (void)sig; g_quit = 1; }

static int W, H, STRIDE, BPP;
static unsigned char *FB;

static void put_gray(int x, int y, unsigned char v) {
    if (x < 0 || y < 0 || x >= W || y >= H) return;
    unsigned char *p = FB + (size_t)y * STRIDE + (size_t)x * BPP;
    if (BPP == 4) {
        p[0] = v; p[1] = v; p[2] = v; p[3] = 0xFF;
    } else {
        memset(p, v, BPP);
    }
}

static void clear_white(void) {
    memset(FB, 0xFF, (size_t)STRIDE * H);
}

static int open_input(const char *needle) {
    char path[64], name[128], lower[128];
    for (int i = 0; i < 8; i++) {
        snprintf(path, sizeof path, "/sys/class/input/event%d/device/name", i);
        FILE *f = fopen(path, "r");
        if (!f) continue;
        if (!fgets(name, sizeof name, f)) { fclose(f); continue; }
        fclose(f);
        memset(lower, 0, sizeof lower);
        for (size_t j = 0; j < sizeof lower - 1 && name[j]; j++)
            lower[j] = (name[j] >= 'A' && name[j] <= 'Z') ? name[j] + 32 : name[j];
        if (!strstr(lower, needle)) continue;
        snprintf(path, sizeof path, "/dev/input/event%d", i);
        int fd = open(path, O_RDONLY | O_NONBLOCK);
        if (fd >= 0) {
            int one = 1;
            ioctl(fd, EVIOCGRAB, &one);
            fprintf(stderr, "image_demo: %s -> %s\n", needle, path);
        }
        return fd;
    }
    return -1;
}

static void drain_inputs(int pwr_fd, int touch_fd) {
    struct input_event evs[64];
    if (pwr_fd >= 0) {
        ssize_t n;
        while ((n = read(pwr_fd, evs, sizeof evs)) > 0) {
            for (int i = 0; i < (int)(n / sizeof(struct input_event)); i++)
                if (evs[i].type == EV_KEY && evs[i].code == KEY_POWER && evs[i].value == 1)
                    g_quit = 1;
        }
    }
    static int slot_active[MAX_SLOTS] = {0};
    static int cur_slot = 0;
    if (touch_fd >= 0) {
        ssize_t n;
        while ((n = read(touch_fd, evs, sizeof evs)) > 0) {
            for (int i = 0; i < (int)(n / sizeof(struct input_event)); i++) {
                struct input_event *e = &evs[i];
                if (e->type == EV_ABS && e->code == ABS_MT_SLOT) {
                    cur_slot = e->value;
                    if (cur_slot < 0 || cur_slot >= MAX_SLOTS) cur_slot = 0;
                } else if (e->type == EV_ABS && e->code == ABS_MT_TRACKING_ID) {
                    slot_active[cur_slot] = (e->value != -1);
                    int fingers = 0;
                    for (int s = 0; s < MAX_SLOTS; s++) fingers += slot_active[s];
                    if (fingers >= 5) g_quit = 1;
                }
            }
        }
    }
}

static int clamp255(int v) {
    return v < 0 ? 0 : (v > 255 ? 255 : v);
}

static int adjusted_luma(QRgb px) {
    int r = qRed(px), g = qGreen(px), b = qBlue(px);
    int l = (r * 30 + g * 59 + b * 11) / 100;
    // Gentle contrast boost around mid-gray. Helps maps/line art survive
    // dithering without turning photos into solid mud.
    l = 128 + (l - 128) * 125 / 100;
    return clamp255(l);
}

static unsigned char ordered_bw(int luma, int x, int y) {
    static const int bayer8[8][8] = {
        { 0,48,12,60, 3,51,15,63}, {32,16,44,28,35,19,47,31},
        { 8,56, 4,52,11,59, 7,55}, {40,24,36,20,43,27,39,23},
        { 2,50,14,62, 1,49,13,61}, {34,18,46,30,33,17,45,29},
        {10,58, 6,54, 9,57, 5,53}, {42,26,38,22,41,25,37,21},
    };
    // Threshold 0..255. Darker luma creates more black pixels.
    int threshold = (bayer8[y & 7][x & 7] * 255 + 31) / 63;
    return luma < threshold ? 0x00 : 0xFF;
}

static void blit_image(const QImage &src, const char *mode, const char *render) {
    clear_white();

    Qt::AspectRatioMode aspect = Qt::KeepAspectRatio;
    if (strcmp(mode, "fill") == 0) aspect = Qt::KeepAspectRatioByExpanding;
    if (strcmp(mode, "stretch") == 0) aspect = Qt::IgnoreAspectRatio;

    QImage scaled = src.convertToFormat(QImage::Format_RGB32)
        .scaled(W, H, aspect, Qt::SmoothTransformation)
        .convertToFormat(QImage::Format_RGB32);

    int ox = (W - scaled.width()) / 2;
    int oy = (H - scaled.height()) / 2;

    // If fill expanded beyond the screen, center-crop while copying.
    int sx0 = ox < 0 ? -ox : 0;
    int sy0 = oy < 0 ? -oy : 0;
    int dx0 = ox > 0 ? ox : 0;
    int dy0 = oy > 0 ? oy : 0;
    int cw = qMin(W - dx0, scaled.width() - sx0);
    int ch = qMin(H - dy0, scaled.height() - sy0);

    if (strcmp(render, "fs") == 0) {
        // Floyd-Steinberg error diffusion. Also pure black/white; more organic
        // than ordered dithering, but noisier for slow e-ink refreshes.
        std::vector<int> lum(cw * ch);
        for (int y = 0; y < ch; y++) {
            const QRgb *row = (const QRgb *)scaled.constScanLine(sy0 + y);
            for (int x = 0; x < cw; x++) lum[y * cw + x] = adjusted_luma(row[sx0 + x]);
        }
        for (int y = 0; y < ch; y++) {
            for (int x = 0; x < cw; x++) {
                int old = lum[y * cw + x];
                int out = old < 128 ? 0 : 255;
                int err = old - out;
                put_gray(dx0 + x, dy0 + y, (unsigned char)out);
                if (x + 1 < cw) lum[y * cw + x + 1] += err * 7 / 16;
                if (y + 1 < ch) {
                    if (x > 0) lum[(y + 1) * cw + x - 1] += err * 3 / 16;
                    lum[(y + 1) * cw + x] += err * 5 / 16;
                    if (x + 1 < cw) lum[(y + 1) * cw + x + 1] += err / 16;
                }
            }
        }
        return;
    }

    for (int y = 0; y < ch; y++) {
        const QRgb *row = (const QRgb *)scaled.constScanLine(sy0 + y);
        for (int x = 0; x < cw; x++) {
            int luma = adjusted_luma(row[sx0 + x]);
            unsigned char v;
            if (strcmp(render, "gray") == 0) {
                v = (unsigned char)luma;
            } else if (strcmp(render, "threshold") == 0) {
                v = luma < 150 ? 0x00 : 0xFF;
            } else {
                v = ordered_bw(luma, dx0 + x, dy0 + y);
            }
            put_gray(dx0 + x, dy0 + y, v);
        }
    }
}

int main(int argc, char **argv) {
    signal(SIGTERM, on_term);
    signal(SIGINT, on_term);

    if (argc < 2) {
        fprintf(stderr, "usage: image_demo /path/to/image.png [fit|fill|stretch] [ordered|fs|threshold|gray]\n");
        return 2;
    }
    const char *mode = argc >= 3 ? argv[2] : "fit";
    const char *render = argc >= 4 ? argv[3] : "ordered";

    if (quill_init() != 0) {
        fprintf(stderr, "image_demo: quill_init failed\n");
        return 1;
    }
    W = quill_width();
    H = quill_height();
    STRIDE = quill_stride();
    BPP = STRIDE / (W > 0 ? W : 1);
    FB = quill_buffer();
    fprintf(stderr, "image_demo: %dx%d stride %d bpp %d fmt %d\n", W, H, STRIDE, BPP, quill_format());
    if (!FB || W <= 0) return 1;

    QImage img(argv[1]);
    if (img.isNull()) {
        fprintf(stderr, "image_demo: could not load image: %s\n", argv[1]);
        return 1;
    }
    fprintf(stderr, "image_demo: loaded %s (%dx%d), mode=%s render=%s\n", argv[1], img.width(), img.height(), mode, render);

    blit_image(img, mode, render);
    quill_swap(0, 0, W, H, 3, 1);

    int pwr_fd = open_input("powerkey");
    int touch_fd = open_input("touch");
    struct pollfd pfds[2] = {{.fd = pwr_fd, .events = POLLIN}, {.fd = touch_fd, .events = POLLIN}};
    while (!g_quit) {
        poll(pfds, 2, 100);
        drain_inputs(pwr_fd, touch_fd);
        quill_process_events();
    }

    fprintf(stderr, "image_demo: bye\n");
    return 0;
}
