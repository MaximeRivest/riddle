#define EPFB_INTERNAL
#include "epframebuffer.h"
#include <map>
#include <iostream>

#include <stdlib.h>
#include <dlfcn.h>

#define ORG(x, ret) ret (*org)(...) = NULL; if(org == NULL) org = (ret (*)(...)) dlsym(RTLD_NEXT, x) 

enum class ImagePtrManagement {
    SWTCON_TypeA, SWTCON_TypeB
};

static QImage *framebufferA, *framebufferB;
static EPFramebuffer *_instance;

static int state = 0;
static std::map<QImage *, ImagePtrManagement> pointerTracker;

static QImage **getGlobal(ImagePtrManagement value) {
    switch(value) {
        case ImagePtrManagement::SWTCON_TypeA:
            return &framebufferA;
        case ImagePtrManagement::SWTCON_TypeB:
            return &framebufferB;
        default:
            return NULL;
    }
}

static void extractPointers() {
    framebufferA = framebufferB = NULL;
    for(const auto &ref : pointerTracker) {
        QImage **img = getGlobal(ref.second);
        if(*img != NULL) {
            std::cerr << "[epaper-framebuffer]: Error: More than once instance of the same type of framebuffer present after contructor terminated!" << std::endl;
            abort();
        }
        *img = ref.first;
    }
    if(!framebufferA || !framebufferB) {
        std::cerr << "[epaper-framebuffer]: Error: One or more framebuffer addresses not set!" << std::endl;
        abort();
    }
}

EPFramebuffer *EPFramebuffer::createControlledInstance() {
    if(state == 2) return _instance;

    state = 1;
    _instance = EPFramebuffer::instance();
    state = 2;
    extractPointers();
    return _instance;
}

extern "C" void _ZN6QImageC1EPhiixNS_6FormatEPFvPvES2_(QImage *that, char *param_1, int param_2, int param_3, long long param_4, int param_5, void* param_6, void * param_7) {
    ORG("_ZN6QImageC1EPhiixNS_6FormatEPFvPvES2_", void);
    org(that, param_1, param_2, param_3, param_4, param_5, param_6, param_7);
    if(state != 1) return;
    // Creating constructor - check what we're creating.
    if(param_5 == 4 || param_5 == 7) {
        pointerTracker[that] = ImagePtrManagement::SWTCON_TypeA;
        std::cerr << "Framebuffer A set!" << std::endl;
    } else if(param_5 == 0x18) {
        pointerTracker[that] = ImagePtrManagement::SWTCON_TypeB;
        std::cerr << "Framebuffer B set!" << std::endl;
    }
}

extern "C" void _ZN6QImageC1ERKS_(QImage *that, QImage *old) {
    ORG("_ZN6QImageC1ERKS_", void);

    // Copying constructor - copy the pointer
    org(that, old);
    if(state != 1) return;
    auto pos = pointerTracker.find(old);
    if(pos != pointerTracker.end()) {
        std::cerr << "Framebuffer cloned from " << old << " to " << that << std::endl;
        pointerTracker[that] = pos->second;
    }
}

extern "C" void _ZN6QImageaSERKS_(QImage *that, QImage *old) {
    ORG("_ZN6QImageaSERKS_", void);

    // Copying constructor - copy the pointer
    org(that, old);
    if(state != 1) return;
    auto pos = pointerTracker.find(old);
    if(pos != pointerTracker.end()) {
        std::cerr << "Framebuffer cloned from " << old << " to " << that << std::endl;
        pointerTracker[that] = pos->second;
    }
}

extern "C" void _ZN6QImageD1Ev(QImage *that) {
    ORG("_ZN6QImageD1Ev", void);
    org(that);

    if(state != 1) return;
    // Destructor - remove the tracked pointer.
    auto pos = pointerTracker.find(that);
    if(pos != pointerTracker.end()) {
        std::cerr << "Framebuffer deleted " << that << std::endl;
        pointerTracker.erase(pos);
    }
}

QImage *EPFramebuffer::getAuxFramebuffer() {
    return framebufferA;
}

QImage *EPFramebuffer::getMainFramebuffer() {
    return framebufferB;
}
