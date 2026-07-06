// image_anim_demo: black/white regional animation experiment for quill takeover.
// Usage: image_anim_demo /path/to/image.png [fit|fill|stretch]
// Renders a dithered image, then fades small regions away/in using only partial
// updates. Exit: power button, 5-finger tap, or SIGTERM.

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

struct input_event { struct timeval time; uint16_t type; uint16_t code; int32_t value; };
static volatile sig_atomic_t g_quit = 0;
static void on_term(int sig) { (void)sig; g_quit = 1; }

static int W, H, STRIDE, BPP;
static unsigned char *FB;
static std::vector<unsigned char> BASE;

static long long now_ms(void) {
    struct timeval tv; gettimeofday(&tv, NULL);
    return (long long)tv.tv_sec * 1000 + tv.tv_usec / 1000;
}

static void put_gray(int x, int y, unsigned char v) {
    if (x < 0 || y < 0 || x >= W || y >= H) return;
    unsigned char *p = FB + (size_t)y * STRIDE + (size_t)x * BPP;
    if (BPP == 4) { p[0] = v; p[1] = v; p[2] = v; p[3] = 0xFF; }
    else memset(p, v, BPP);
}

static int clamp255(int v) { return v < 0 ? 0 : (v > 255 ? 255 : v); }
static int adjusted_luma(QRgb px) {
    int l = (qRed(px) * 30 + qGreen(px) * 59 + qBlue(px) * 11) / 100;
    return clamp255(128 + (l - 128) * 125 / 100);
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
static uint32_t hashxy(int x, int y, int salt) {
    uint32_t h = (uint32_t)x * 0x9E3779B1u ^ (uint32_t)y * 0x85EBCA6Bu ^ (uint32_t)salt * 0xC2B2AE35u;
    h ^= h >> 13; h *= 0x27d4eb2du; return h ^ (h >> 15);
}

static void base_to_fb(void) {
    for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) put_gray(x, y, BASE[y * W + x]);
}

static void render_base(const QImage &src, const char *mode) {
    BASE.assign((size_t)W * H, 0xFF);
    Qt::AspectRatioMode aspect = Qt::KeepAspectRatio;
    if (strcmp(mode, "fill") == 0) aspect = Qt::KeepAspectRatioByExpanding;
    if (strcmp(mode, "stretch") == 0) aspect = Qt::IgnoreAspectRatio;
    QImage scaled = src.convertToFormat(QImage::Format_RGB32)
        .scaled(W, H, aspect, Qt::SmoothTransformation)
        .convertToFormat(QImage::Format_RGB32);
    int ox = (W - scaled.width()) / 2, oy = (H - scaled.height()) / 2;
    int sx0 = ox < 0 ? -ox : 0, sy0 = oy < 0 ? -oy : 0;
    int dx0 = ox > 0 ? ox : 0, dy0 = oy > 0 ? oy : 0;
    int cw = qMin(W - dx0, scaled.width() - sx0), ch = qMin(H - dy0, scaled.height() - sy0);
    for (int y = 0; y < ch; y++) {
        const QRgb *row = (const QRgb *)scaled.constScanLine(sy0 + y);
        for (int x = 0; x < cw; x++) {
            int xx = dx0 + x, yy = dy0 + y;
            BASE[yy * W + xx] = ordered_bw(adjusted_luma(row[sx0 + x]), xx, yy);
        }
    }
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
        if (fd >= 0) { int one = 1; ioctl(fd, EVIOCGRAB, &one); fprintf(stderr, "image_anim_demo: %s -> %s\n", needle, path); }
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

static void fade_rect(int x0, int y0, int rw, int rh, int phase, int phases, int salt, bool reveal) {
    for (int y = y0; y < y0 + rh && y < H; y++) for (int x = x0; x < x0 + rw && x < W; x++) {
        int keep = (int)(hashxy(x, y, salt) % phases);
        bool show_base = reveal ? (keep <= phase) : (keep > phase);
        put_gray(x, y, show_base ? BASE[y * W + x] : 0xFF);
    }
}

int main(int argc, char **argv) {
    signal(SIGTERM, on_term); signal(SIGINT, on_term);
    if (argc < 2) { fprintf(stderr, "usage: image_anim_demo /path/to/image.png [fit|fill|stretch]\n"); return 2; }
    const char *mode = argc >= 3 ? argv[2] : "fit";
    if (quill_init() != 0) { fprintf(stderr, "image_anim_demo: quill_init failed\n"); return 1; }
    W = quill_width(); H = quill_height(); STRIDE = quill_stride(); BPP = STRIDE / (W > 0 ? W : 1); FB = quill_buffer();
    fprintf(stderr, "image_anim_demo: %dx%d stride %d bpp %d fmt %d\n", W, H, STRIDE, BPP, quill_format());
    QImage img(argv[1]); if (img.isNull()) { fprintf(stderr, "image_anim_demo: could not load %s\n", argv[1]); return 1; }

    render_base(img, mode); base_to_fb(); quill_swap(0, 0, W, H, 3, 1);
    int pwr_fd = open_input("powerkey"), touch_fd = open_input("touch");
    struct pollfd pfds[2] = {{.fd = pwr_fd, .events = POLLIN}, {.fd = touch_fd, .events = POLLIN}};

    const int phases = 12;
    int rw = W / 4, rh = H / 6;
    int rects[][2] = {{W/10,H/6},{W*58/100,H/6},{W/7,H*56/100},{W*56/100,H*54/100},{W*38/100,H*35/100}};
    int nrects = (int)(sizeof rects / sizeof rects[0]);
    int cycle = 0;
    while (!g_quit) {
        int a = cycle % nrects, b = (cycle + 2) % nrects;
        long long t0 = now_ms();
        for (int p = 0; p < phases && !g_quit; p++) {
            long long fs = now_ms();
            fade_rect(rects[a][0], rects[a][1], rw, rh, p, phases, cycle * 17 + 1, false);
            fade_rect(rects[b][0], rects[b][1], rw, rh, p, phases, cycle * 17 + 9, true);
            quill_swap(rects[a][0], rects[a][1], rw, rh, 0, 0);
            quill_swap(rects[b][0], rects[b][1], rw, rh, 0, 0);
            quill_process_events();
            fprintf(stderr, "image_anim_demo: frame %d swap+draw=%lldms\n", p, now_ms() - fs);
            poll(pfds, 2, 70);
            drain_inputs(pwr_fd, touch_fd);
        }
        fprintf(stderr, "image_anim_demo: cycle %d %.2f fps-ish\n", cycle, phases * 1000.0 / (double)(now_ms() - t0 + 1));
        poll(pfds, 2, 450); drain_inputs(pwr_fd, touch_fd);
        // Repaint full base every few cycles to reset accumulated ghosting.
        if (++cycle % 6 == 0) { base_to_fb(); quill_swap(0, 0, W, H, 3, 1); }
    }
    fprintf(stderr, "image_anim_demo: bye\n");
    return 0;
}
