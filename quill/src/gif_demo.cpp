// gif_demo: render an animated GIF/WebP-ish image through quill takeover.
// Usage: gif_demo /path/to/anim.gif [fit|fill|stretch] [ordered|threshold|gray]
// Preloads frames, dithers to black/white, then updates only changed bbox.
// Exit: power button, 5-finger tap, or SIGTERM.

#include <QImage>
#include <QImageReader>
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

struct input_event { struct timeval time; uint16_t type; uint16_t code; int32_t value; };
struct Frame { std::vector<unsigned char> px; int delay_ms; };

static volatile sig_atomic_t g_quit = 0;
static void on_term(int sig) { (void)sig; g_quit = 1; }
static int W, H, STRIDE, BPP, ROI_X, ROI_Y, ROI_W, ROI_H;
static unsigned char *FB;

static void put_gray(int x, int y, unsigned char v) {
    if (x < 0 || y < 0 || x >= W || y >= H) return;
    unsigned char *p = FB + (size_t)y * STRIDE + (size_t)x * BPP;
    if (BPP == 4) { p[0] = v; p[1] = v; p[2] = v; p[3] = 0xFF; }
    else memset(p, v, BPP);
}
static int clamp255(int v) { return v < 0 ? 0 : (v > 255 ? 255 : v); }
static int adjusted_luma(QRgb px) {
    int l = (qRed(px) * 30 + qGreen(px) * 59 + qBlue(px) * 11) / 100;
    return clamp255(128 + (l - 128) * 130 / 100);
}
static unsigned char ordered_bw(int luma, int x, int y) {
    static const int bayer8[8][8] = {
        { 0,48,12,60, 3,51,15,63}, {32,16,44,28,35,19,47,31},
        { 8,56, 4,52,11,59, 7,55}, {40,24,36,20,43,27,39,23},
        { 2,50,14,62, 1,49,13,61}, {34,18,46,30,33,17,45,29},
        {10,58, 6,54, 9,57, 5,53}, {42,26,38,22,41,25,37,21},
    };
    int threshold = (bayer8[y & 7][x & 7] * 255 + 31) / 63;
    return luma < threshold ? 0x00 : 0xFF;
}

static int open_input(const char *needle) {
    char path[64], name[128], lower[128];
    for (int i = 0; i < 8; i++) {
        snprintf(path, sizeof path, "/sys/class/input/event%d/device/name", i);
        FILE *f = fopen(path, "r"); if (!f) continue;
        if (!fgets(name, sizeof name, f)) { fclose(f); continue; }
        fclose(f); memset(lower, 0, sizeof lower);
        for (size_t j = 0; j < sizeof lower - 1 && name[j]; j++) lower[j] = (name[j] >= 'A' && name[j] <= 'Z') ? name[j] + 32 : name[j];
        if (!strstr(lower, needle)) continue;
        snprintf(path, sizeof path, "/dev/input/event%d", i);
        int fd = open(path, O_RDONLY | O_NONBLOCK);
        if (fd >= 0) { int one = 1; ioctl(fd, EVIOCGRAB, &one); fprintf(stderr, "gif_demo: %s -> %s\n", needle, path); }
        return fd;
    }
    return -1;
}
static void drain_inputs(int pwr_fd, int touch_fd) {
    struct input_event evs[64];
    if (pwr_fd >= 0) { ssize_t n; while ((n = read(pwr_fd, evs, sizeof evs)) > 0)
        for (int i = 0; i < (int)(n / sizeof(struct input_event)); i++)
            if (evs[i].type == EV_KEY && evs[i].code == KEY_POWER && evs[i].value == 1) g_quit = 1; }
    static int slot_active[MAX_SLOTS] = {0}; static int cur_slot = 0;
    if (touch_fd >= 0) { ssize_t n; while ((n = read(touch_fd, evs, sizeof evs)) > 0) {
        for (int i = 0; i < (int)(n / sizeof(struct input_event)); i++) {
            struct input_event *e = &evs[i];
            if (e->type == EV_ABS && e->code == ABS_MT_SLOT) { cur_slot = e->value; if (cur_slot < 0 || cur_slot >= MAX_SLOTS) cur_slot = 0; }
            else if (e->type == EV_ABS && e->code == ABS_MT_TRACKING_ID) {
                slot_active[cur_slot] = (e->value != -1); int fingers = 0;
                for (int s = 0; s < MAX_SLOTS; s++) fingers += slot_active[s];
                if (fingers >= 5) g_quit = 1;
            }
        }
    }}
}

static void compute_roi(int iw, int ih, const char *mode) {
    if (strcmp(mode, "stretch") == 0) { ROI_X = 0; ROI_Y = 0; ROI_W = W; ROI_H = H; return; }
    double sx = (double)W / iw, sy = (double)H / ih;
    double s = strcmp(mode, "fill") == 0 ? (sx > sy ? sx : sy) : (sx < sy ? sx : sy);
    ROI_W = (int)(iw * s + 0.5); ROI_H = (int)(ih * s + 0.5);
    if (ROI_W > W) ROI_W = W; if (ROI_H > H) ROI_H = H;
    ROI_X = (W - ROI_W) / 2; ROI_Y = (H - ROI_H) / 2;
}

static Frame render_frame(const QImage &img, int delay_ms, const char *mode, const char *render) {
    QImage scaled = img.convertToFormat(QImage::Format_RGB32)
        .scaled(ROI_W, ROI_H, strcmp(mode, "stretch") == 0 ? Qt::IgnoreAspectRatio : Qt::KeepAspectRatioByExpanding, Qt::SmoothTransformation)
        .convertToFormat(QImage::Format_RGB32);
    int sx0 = qMax(0, (scaled.width() - ROI_W) / 2), sy0 = qMax(0, (scaled.height() - ROI_H) / 2);
    Frame f; f.delay_ms = delay_ms > 0 ? delay_ms : 100; if (f.delay_ms < 90) f.delay_ms = 90;
    f.px.assign((size_t)ROI_W * ROI_H, 0xFF);
    for (int y = 0; y < ROI_H; y++) {
        const QRgb *row = (const QRgb *)scaled.constScanLine(sy0 + y);
        for (int x = 0; x < ROI_W; x++) {
            int l = adjusted_luma(row[sx0 + x]);
            if (strcmp(render, "gray") == 0) f.px[y * ROI_W + x] = (unsigned char)l;
            else if (strcmp(render, "threshold") == 0) f.px[y * ROI_W + x] = l < 150 ? 0x00 : 0xFF;
            else f.px[y * ROI_W + x] = ordered_bw(l, ROI_X + x, ROI_Y + y);
        }
    }
    return f;
}

int main(int argc, char **argv) {
    signal(SIGTERM, on_term); signal(SIGINT, on_term);
    if (argc < 2) { fprintf(stderr, "usage: gif_demo /path/to/anim.gif [fit|fill|stretch] [ordered|threshold|gray]\n"); return 2; }
    const char *mode = argc >= 3 ? argv[2] : "fit";
    const char *render = argc >= 4 ? argv[3] : "ordered";
    if (quill_init() != 0) { fprintf(stderr, "gif_demo: quill_init failed\n"); return 1; }
    W = quill_width(); H = quill_height(); STRIDE = quill_stride(); BPP = STRIDE / (W > 0 ? W : 1); FB = quill_buffer();
    fprintf(stderr, "gif_demo: %dx%d stride %d bpp %d fmt %d\n", W, H, STRIDE, BPP, quill_format());

    QImageReader reader(argv[1]);
    if (!reader.canRead()) { fprintf(stderr, "gif_demo: cannot read %s: %s\n", argv[1], reader.errorString().toUtf8().constData()); return 1; }
    QSize sz = reader.size(); if (!sz.isValid()) { QImage first = reader.read(); sz = first.size(); reader.setFileName(argv[1]); }
    compute_roi(sz.width(), sz.height(), mode);
    fprintf(stderr, "gif_demo: source=%dx%d roi=%d,%d %dx%d mode=%s render=%s\n", sz.width(), sz.height(), ROI_X, ROI_Y, ROI_W, ROI_H, mode, render);

    std::vector<Frame> frames;
    while (reader.canRead()) {
        int delay = reader.nextImageDelay();
        QImage img = reader.read();
        if (img.isNull()) break;
        frames.push_back(render_frame(img, delay, mode, render));
        if (frames.size() >= 180) break; // avoid pathological memory use
    }
    if (frames.empty()) { fprintf(stderr, "gif_demo: no frames loaded\n"); return 1; }
    fprintf(stderr, "gif_demo: loaded %zu frames\n", frames.size());

    memset(FB, 0xFF, (size_t)STRIDE * H);
    std::vector<unsigned char> prev((size_t)ROI_W * ROI_H, 0xFF);
    int pwr_fd = open_input("powerkey"), touch_fd = open_input("touch");
    struct pollfd pfds[2] = {{.fd = pwr_fd, .events = POLLIN}, {.fd = touch_fd, .events = POLLIN}};

    size_t i = 0, shown = 0;
    while (!g_quit) {
        Frame &f = frames[i];
        int x0 = ROI_W, y0 = ROI_H, x1 = -1, y1 = -1;
        for (int y = 0; y < ROI_H; y++) for (int x = 0; x < ROI_W; x++) {
            unsigned char v = f.px[y * ROI_W + x];
            if (v != prev[y * ROI_W + x]) {
                put_gray(ROI_X + x, ROI_Y + y, v);
                prev[y * ROI_W + x] = v;
                if (x < x0) x0 = x; if (y < y0) y0 = y; if (x > x1) x1 = x; if (y > y1) y1 = y;
            }
        }
        if (x1 >= x0) quill_swap(ROI_X + x0, ROI_Y + y0, x1 - x0 + 1, y1 - y0 + 1, 0, 0);
        quill_process_events();
        if (++shown % 120 == 0) quill_swap(0, 0, W, H, 3, 1);
        poll(pfds, 2, f.delay_ms);
        drain_inputs(pwr_fd, touch_fd);
        i = (i + 1) % frames.size();
    }
    fprintf(stderr, "gif_demo: bye\n");
    return 0;
}
