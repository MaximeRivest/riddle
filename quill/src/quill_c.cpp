// quill_c: C ABI over the vendor e-ink engine (libqsgepaper's EPFramebuffer,
// accessed via asivery's epfb-re shim). Runs with xochitl STOPPED — this
// process becomes the display driver.
//
// The engine wants a Qt context; we create a QCoreApplication but drive our
// own loop — swapBuffers() is synchronous enough for ink.

#include "epframebuffer.h"
#include <QCoreApplication>
#include <QImage>
#include <cstring>
#include <cstdio>

static QCoreApplication *g_app = nullptr;
static EPFramebuffer *g_fb = nullptr;
static QImage *g_aux = nullptr;

extern "C" {

// Returns 0 on success. After this, quill_buffer()/quill_swap() are usable.
int quill_init() {
    if (g_fb) return 0;
    static int argc = 1;
    static char arg0[] = "quill";
    static char *argv[] = {arg0, nullptr};
    g_app = new QCoreApplication(argc, argv);
    g_fb = EPFramebuffer::createControlledInstance();
    if (!g_fb) return 1;
    g_aux = g_fb->getAuxFramebuffer();
    if (!g_aux) return 2;
    fprintf(stderr, "quill: aux framebuffer %dx%d format=%d bpl=%lld\n",
            g_aux->width(), g_aux->height(), (int)g_aux->format(),
            (long long)g_aux->bytesPerLine());
    return 0;
}

// Geometry of the drawing buffer.
int quill_width()  { return g_aux ? g_aux->width() : 0; }
int quill_height() { return g_aux ? g_aux->height() : 0; }
int quill_stride() { return g_aux ? (int)g_aux->bytesPerLine() : 0; }
int quill_format() { return g_aux ? (int)g_aux->format() : -1; }

// Direct pointer into the aux framebuffer pixels.
unsigned char *quill_buffer() {
    return g_aux ? g_aux->bits() : nullptr;
}

// Push a region to glass. mode: 0=fastest(DU-ish) 1=fast 3=medium 4=full-quality.
// full_refresh != 0 forces a flashing clear of the region.
unsigned long quill_swap(int x, int y, int w, int h, int mode, int full_refresh) {
    if (!g_fb) return 0;
    QFlags<EPFramebuffer::UpdateFlag> flags = full_refresh
        ? EPFramebuffer::UpdateFlag::CompleteRefresh
        : EPFramebuffer::UpdateFlag::NoRefresh;
    return g_fb->swapBuffers(QRect(x, y, w, h), EPContentType::Mono,
                             (EPScreenMode)mode, flags);
}

void quill_process_events() {
    if (g_app) QCoreApplication::processEvents();
}

} // extern "C"
