// scribble: C1 milestone demo. Raw pen -> vendor e-ink engine, xochitl stopped.
// Draw with the pen (pressure-width ink), eraser tip erases, stylus writes.
// Exit: hold the pen's side button OR press the power button OR SIGTERM.
//
// This measures the floor: evdev-to-glass with nothing in between.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <signal.h>
#include <stdint.h>
#include <sys/ioctl.h>
#include <sys/time.h>
#include <poll.h>

// quill_c API
extern int quill_init(void);
extern int quill_width(void);
extern int quill_height(void);
extern int quill_stride(void);
extern int quill_format(void);
extern unsigned char *quill_buffer(void);
extern unsigned long quill_swap(int x, int y, int w, int h, int mode, int full);
extern void quill_process_events(void);

#define EV_SYN 0
#define EV_KEY 1
#define EV_ABS 3
#define ABS_X 0
#define ABS_Y 1
#define ABS_PRESSURE 24
#define BTN_TOOL_PEN 320
#define BTN_TOOL_RUBBER 321
#define BTN_STYLUS 331
#define BTN_TOUCH 330
#define KEY_POWER 116
#define EVIOCGRAB 0x40044590

#define DIGI_MAX_X 11180
#define DIGI_MAX_Y 15340

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

// Assume grayscale-ish linear formats; QImage formats: 4=RGB32, 7=? , 24=Grayscale8.
static void put_px(int x, int y, int black) {
    if (x < 0 || y < 0 || x >= W || y >= H) return;
    unsigned char v = black ? 0x00 : 0xFF;
    unsigned char *p = FB + (size_t)y * STRIDE + (size_t)x * BPP;
    memset(p, v, BPP);
    if (BPP == 4) p[3] = 0xFF; // alpha
}

static void stamp(int cx, int cy, int r, int black) {
    for (int dy = -r; dy <= r; dy++)
        for (int dx = -r; dx <= r; dx++)
            if (dx * dx + dy * dy <= r * r)
                put_px(cx + dx, cy + dy, black);
}

static void line(int x0, int y0, int x1, int y1, int r, int black) {
    int dx = abs(x1 - x0), dy = abs(y1 - y0);
    int steps = (dx > dy ? dx : dy);
    if (steps < 1) steps = 1;
    for (int i = 0; i <= steps; i++)
        stamp(x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps, r, black);
}

static int open_input(const char *needle) {
    char path[64], name[128];
    for (int i = 0; i < 8; i++) {
        snprintf(path, sizeof path, "/sys/class/input/event%d/device/name", i);
        FILE *f = fopen(path, "r");
        if (!f) continue;
        if (fgets(name, sizeof name, f) && strstr(name, needle)) {
            fclose(f);
            snprintf(path, sizeof path, "/dev/input/event%d", i);
            int fd = open(path, O_RDONLY | O_NONBLOCK);
            if (fd >= 0) {
                int one = 1;
                ioctl(fd, EVIOCGRAB, &one);
                fprintf(stderr, "scribble: %s -> %s\n", needle, path);
            }
            return fd;
        }
        fclose(f);
    }
    return -1;
}

int main(void) {
    signal(SIGTERM, on_term);
    signal(SIGINT, on_term);

    if (quill_init() != 0) {
        fprintf(stderr, "scribble: quill_init failed\n");
        return 1;
    }
    W = quill_width();
    H = quill_height();
    STRIDE = quill_stride();
    BPP = STRIDE / (W > 0 ? W : 1);
    FB = quill_buffer();
    fprintf(stderr, "scribble: %dx%d stride %d bpp %d fmt %d\n", W, H, STRIDE, BPP, quill_format());
    if (!FB || W <= 0) return 1;

    // White page, full flashing refresh once.
    memset(FB, 0xFF, (size_t)STRIDE * H);
    quill_swap(0, 0, W, H, /*Quality3*/ 3, /*full*/ 1);

    int pen_fd = open_input("marker");
    int pwr_fd = open_input("powerkey");
    if (pen_fd < 0) {
        fprintf(stderr, "scribble: no pen!\n");
        return 1;
    }

    int rx = 0, ry = 0, pressure = 0, touching = 0, eraser = 0, side_btn = 0;
    int lx = -1, ly = -1;
    int have = 0;
    // dirty rect
    int dx0 = 1 << 30, dy0 = 1 << 30, dx1 = -1, dy1 = -1;
    struct timeval last_flush = {0, 0};

    struct pollfd pfds[2] = {
        {.fd = pen_fd, .events = POLLIN},
        {.fd = pwr_fd, .events = POLLIN},
    };

    while (!g_quit) {
        poll(pfds, pwr_fd >= 0 ? 2 : 1, 5);
        struct input_event evs[64];

        if (pwr_fd >= 0) {
            ssize_t n = read(pwr_fd, evs, sizeof evs);
            for (int i = 0; i < (int)(n / sizeof(struct input_event)); i++)
                if (evs[i].type == EV_KEY && evs[i].code == KEY_POWER && evs[i].value == 1)
                    g_quit = 1;
        }

        ssize_t n = read(pen_fd, evs, sizeof evs);
        int frames = (n > 0) ? (int)(n / sizeof(struct input_event)) : 0;
        for (int i = 0; i < frames; i++) {
            struct input_event *e = &evs[i];
            if (e->type == EV_ABS && e->code == ABS_X) { rx = e->value; have = 1; }
            else if (e->type == EV_ABS && e->code == ABS_Y) { ry = e->value; have = 1; }
            else if (e->type == EV_ABS && e->code == ABS_PRESSURE) { pressure = e->value; have = 1; }
            else if (e->type == EV_KEY && e->code == BTN_TOOL_RUBBER) eraser = e->value;
            else if (e->type == EV_KEY && e->code == BTN_STYLUS) side_btn = e->value;
            else if (e->type == EV_KEY && e->code == BTN_TOUCH) { touching = e->value; have = 1; }
            else if (e->type == EV_SYN && have) {
                have = 0;
                int x = (int)((int64_t)rx * (W - 1) / DIGI_MAX_X);
                int y = (int)((int64_t)ry * (H - 1) / DIGI_MAX_Y);
                if (touching && pressure > 40) {
                    int r = eraser ? 22 : 2 + pressure * 3 / 4096;
                    if (lx >= 0) line(lx, ly, x, y, r, !eraser);
                    else stamp(x, y, r, !eraser);
                    int m = r + 2;
                    if (x - m < dx0) dx0 = x - m;
                    if (y - m < dy0) dy0 = y - m;
                    if (x + m > dx1) dx1 = x + m;
                    if (y + m > dy1) dy1 = y + m;
                    if (lx >= 0) {
                        if (lx - m < dx0) dx0 = lx - m;
                        if (ly - m < dy0) dy0 = ly - m;
                        if (lx + m > dx1) dx1 = lx + m;
                        if (ly + m > dy1) dy1 = ly + m;
                    }
                    lx = x; ly = y;
                } else {
                    lx = ly = -1;
                }
                if (side_btn && !touching) g_quit = 1; // side button in hover = quit
            }
        }

        // Flush dirty ink immediately — this is the latency experiment:
        // no coalescing beyond one poll cycle (~5ms).
        if (dx1 >= 0) {
            struct timeval now;
            gettimeofday(&now, NULL);
            long ms = (now.tv_sec - last_flush.tv_sec) * 1000 + (now.tv_usec - last_flush.tv_usec) / 1000;
            if (ms >= 8) {
                if (dx0 < 0) dx0 = 0;
                if (dy0 < 0) dy0 = 0;
                if (dx1 >= W) dx1 = W - 1;
                if (dy1 >= H) dy1 = H - 1;
                quill_swap(dx0, dy0, dx1 - dx0 + 1, dy1 - dy0 + 1, /*fastest*/ 0, 0);
                dx0 = dy0 = 1 << 30;
                dx1 = dy1 = -1;
                last_flush = now;
            }
        }
        quill_process_events();
    }

    fprintf(stderr, "scribble: bye\n");
    return 0;
}
