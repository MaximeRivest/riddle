// map_demo: Marauder's-Map-style e-ink animation demo for quill takeover.
// Draws a static map once, then animates tiny footprint sprites with partial
// updates. Exit: power button, 5-finger tap, or SIGTERM.

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

extern int quill_init(void);
extern int quill_width(void);
extern int quill_height(void);
extern int quill_stride(void);
extern int quill_format(void);
extern unsigned char *quill_buffer(void);
extern unsigned long quill_swap(int x, int y, int w, int h, int mode, int full);
extern void quill_process_events(void);

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

static void fill_rect(int x, int y, int w, int h, unsigned char v) {
    for (int yy = y; yy < y + h; yy++)
        for (int xx = x; xx < x + w; xx++)
            put_gray(xx, yy, v);
}

static void stamp(int cx, int cy, int r, unsigned char v) {
    for (int dy = -r; dy <= r; dy++)
        for (int dx = -r; dx <= r; dx++)
            if (dx * dx + dy * dy <= r * r)
                put_gray(cx + dx, cy + dy, v);
}

static void line(int x0, int y0, int x1, int y1, int r, unsigned char v) {
    int dx = abs(x1 - x0), dy = abs(y1 - y0);
    int steps = dx > dy ? dx : dy;
    if (steps < 1) steps = 1;
    for (int i = 0; i <= steps; i++) {
        int x = x0 + (x1 - x0) * i / steps;
        int y = y0 + (y1 - y0) * i / steps;
        stamp(x, y, r, v);
    }
}

static void room(int x, int y, int w, int h) {
    line(x, y, x + w, y, 3, 0x00);
    line(x + w, y, x + w, y + h, 3, 0x00);
    line(x + w, y + h, x, y + h, 3, 0x00);
    line(x, y + h, x, y, 3, 0x00);
    // lightly aged interior hatch marks
    for (int i = 26; i < w - 20; i += 44) line(x + i, y + 8, x + i - 18, y + 26, 1, 0xB8);
}

static void draw_map(void) {
    memset(FB, 0xFF, (size_t)STRIDE * H);

    int mx = W / 10, my = H / 10;
    int mw = W - 2 * mx, mh = H - 2 * my;

    // parchment border
    line(mx, my, mx + mw, my, 5, 0x00);
    line(mx + mw, my, mx + mw, my + mh, 5, 0x00);
    line(mx + mw, my + mh, mx, my + mh, 5, 0x00);
    line(mx, my + mh, mx, my, 5, 0x00);
    line(mx + 24, my + 24, mx + mw - 24, my + 24, 1, 0x55);
    line(mx + mw - 24, my + 24, mx + mw - 24, my + mh - 24, 1, 0x55);
    line(mx + mw - 24, my + mh - 24, mx + 24, my + mh - 24, 1, 0x55);
    line(mx + 24, my + mh - 24, mx + 24, my + 24, 1, 0x55);

    // rooms and corridors
    room(mx + 100, my + 130, 420, 300);
    room(mx + mw - 560, my + 120, 430, 330);
    room(mx + 180, my + mh - 480, 470, 300);
    room(mx + mw - 640, my + mh - 520, 480, 350);
    room(W / 2 - 230, H / 2 - 190, 460, 380);

    line(mx + 520, my + 280, W / 2 - 230, H / 2, 7, 0x00);
    line(W / 2 + 230, H / 2, mx + mw - 560, my + 280, 7, 0x00);
    line(W / 2, H / 2 + 190, mx + 415, my + mh - 480, 7, 0x00);
    line(W / 2 + 80, H / 2 + 190, mx + mw - 400, my + mh - 520, 7, 0x00);
    line(W / 2 - 40, H / 2 - 190, W / 2 - 40, my + 120, 5, 0x00);

    // title-ish marks without needing a font
    line(mx + 210, my + 70, mx + mw - 210, my + 70, 2, 0x55);
    line(mx + 270, my + 95, mx + mw - 270, my + 95, 1, 0x88);
}

static void draw_foot(int x, int y, int side, unsigned char v) {
    // side: -1 left, +1 right. A heel + toe cluster, rotated-ish by offset.
    stamp(x, y, 8, v);
    stamp(x + side * 8, y - 13, 5, v);
    stamp(x + side * 2, y - 20, 3, v);
    stamp(x + side * 9, y - 22, 3, v);
}

static void foot_bbox(int x, int y, int *rx, int *ry, int *rw, int *rh) {
    *rx = x - 24; *ry = y - 34; *rw = 48; *rh = 54;
    if (*rx < 0) *rx = 0;
    if (*ry < 0) *ry = 0;
    if (*rx + *rw > W) *rw = W - *rx;
    if (*ry + *rh > H) *rh = H - *ry;
}

static int open_input(const char *needle) {
    char path[64], name[128], lower[128];
    for (int i = 0; i < 8; i++) {
        snprintf(path, sizeof path, "/sys/class/input/event%d/device/name", i);
        FILE *f = fopen(path, "r");
        if (!f) continue;
        if (!fgets(name, sizeof name, f)) { fclose(f); continue; }
        fclose(f);
        for (size_t j = 0; j < sizeof lower - 1 && name[j]; j++) {
            lower[j] = (name[j] >= 'A' && name[j] <= 'Z') ? name[j] + 32 : name[j];
            lower[j + 1] = 0;
        }
        if (!strstr(lower, needle)) continue;
        snprintf(path, sizeof path, "/dev/input/event%d", i);
        int fd = open(path, O_RDONLY | O_NONBLOCK);
        if (fd >= 0) {
            int one = 1;
            ioctl(fd, EVIOCGRAB, &one);
            fprintf(stderr, "map_demo: %s -> %s\n", needle, path);
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

int main(void) {
    signal(SIGTERM, on_term);
    signal(SIGINT, on_term);

    if (quill_init() != 0) {
        fprintf(stderr, "map_demo: quill_init failed\n");
        return 1;
    }
    W = quill_width();
    H = quill_height();
    STRIDE = quill_stride();
    BPP = STRIDE / (W > 0 ? W : 1);
    FB = quill_buffer();
    fprintf(stderr, "map_demo: %dx%d stride %d bpp %d fmt %d\n", W, H, STRIDE, BPP, quill_format());
    if (!FB || W <= 0) return 1;

    int pwr_fd = open_input("powerkey");
    int touch_fd = open_input("touch");

    draw_map();
    quill_swap(0, 0, W, H, 3, 1); // one clean, full map reveal

    int path[][2] = {
        {W/2 - 470, H/2 - 30}, {W/2 - 380, H/2 - 20}, {W/2 - 290, H/2 - 5},
        {W/2 - 200, H/2 + 15}, {W/2 - 110, H/2 + 35}, {W/2 - 20, H/2 + 45},
        {W/2 + 70, H/2 + 35}, {W/2 + 160, H/2 + 15}, {W/2 + 250, H/2 - 5},
        {W/2 + 340, H/2 - 22}, {W/2 + 430, H/2 - 35},
    };
    int npath = (int)(sizeof path / sizeof path[0]);
    int step = 0;
    struct pollfd pfds[2] = {{.fd = pwr_fd, .events = POLLIN}, {.fd = touch_fd, .events = POLLIN}};

    while (!g_quit) {
        int x = path[step % npath][0];
        int y = path[step % npath][1] + ((step / npath) % 2) * 110;
        int side = (step % 2) ? 1 : -1;
        int rx, ry, rw, rh;
        foot_bbox(x, y, &rx, &ry, &rw, &rh);

        draw_foot(x, y, side, 0x00);
        quill_swap(rx, ry, rw, rh, 0, 0); // tiny low-latency redraw

        // Let the footprint linger, then erase by redrawing just the map region.
        for (int i = 0; i < 10 && !g_quit; i++) {
            poll(pfds, 2, 45);
            drain_inputs(pwr_fd, touch_fd);
            quill_process_events();
        }
        fill_rect(rx, ry, rw, rh, 0xFF);
        draw_map(); // simple correctness over speed; only swap the old foot box
        quill_swap(rx, ry, rw, rh, 0, 0);

        step++;
        if (step % 28 == 0) quill_swap(0, 0, W, H, 3, 1); // occasional ghost cleanup
    }

    fprintf(stderr, "map_demo: bye\n");
    return 0;
}
