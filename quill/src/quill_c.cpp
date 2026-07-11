// quill_c: C ABI over the vendor e-ink engine (libqsgepaper's EPFramebuffer,
// accessed via asivery's epfb-re shim). Runs with xochitl STOPPED — this
// process becomes the display driver.
//
// The engine wants a Qt context; we create a QCoreApplication but drive our
// own loop — swapBuffers() is synchronous enough for ink.

#include "epframebuffer.h"
#include <QCoreApplication>
#include <QImage>
#include <dlfcn.h>
#include <cstring>
#include <cstdio>

static QCoreApplication *g_app = nullptr;
static EPFramebuffer *g_fb = nullptr;
static QImage *g_aux = nullptr;

// OS 3.27 and earlier:
//   swapBuffers(QRect, EPContentType, EPScreenMode, QFlags<UpdateFlag>)
// OS 3.28+ dropped EPContentType from the QRect overload:
//   swapBuffers(QRect, EPScreenMode, QFlags<UpdateFlag>)
using SwapOldFn = unsigned long (*)(EPFramebuffer *, QRect, EPContentType,
                                    EPScreenMode, EPFramebuffer::UpdateFlag);
using SwapNewFn = unsigned long (*)(EPFramebuffer *, QRect, EPScreenMode,
                                    EPFramebuffer::UpdateFlag);
static SwapOldFn g_swap_old = nullptr;
static SwapNewFn g_swap_new = nullptr;

static void resolve_swap() {
    if (g_swap_old || g_swap_new) return;
    g_swap_new = reinterpret_cast<SwapNewFn>(dlsym(
        RTLD_DEFAULT,
        "_ZN13EPFramebuffer11swapBuffersE5QRect12EPScreenMode6QFlagsINS_10UpdateFlagEE"));
    g_swap_old = reinterpret_cast<SwapOldFn>(dlsym(
        RTLD_DEFAULT,
        "_ZN13EPFramebuffer11swapBuffersE5QRect13EPContentType12EPScreenMode6QFlagsINS_10UpdateFlagEE"));
}

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
    resolve_swap();
    EPFramebuffer::UpdateFlag flag = full_refresh
        ? EPFramebuffer::UpdateFlag::CompleteRefresh
        : EPFramebuffer::UpdateFlag::NoRefresh;
    QRect rect(x, y, w, h);
    EPScreenMode screen = static_cast<EPScreenMode>(mode);
    if (g_swap_new) {
        return g_swap_new(g_fb, rect, screen, flag);
    }
    if (g_swap_old) {
        return g_swap_old(g_fb, rect, EPContentType::Mono, screen, flag);
    }
    fprintf(stderr, "quill: no EPFramebuffer::swapBuffers symbol found\n");
    return 0;
}

void quill_process_events() {
    if (g_app) QCoreApplication::processEvents();
}

} // extern "C"
