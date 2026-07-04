#pragma once
#include <QImage>
#include <QRect>
#include <QFlags>


enum EPScreenMode {
    QualityFastest = 0,
    QualityFast = 1,
    Quality3 = 3,
    QualityFull = 4,
    Quality5 = 5,
};

enum EPContentType {
    Mono = 0,
    Color = 1,
};

class EPFramebuffer {
public:
    enum UpdateFlag {
        NoRefresh = 0,
        CompleteRefresh = 1,
    };
    unsigned long swapBuffers(QRect param_1, EPContentType epct, EPScreenMode type, QFlags<EPFramebuffer::UpdateFlag> flags);
    #ifdef EPFB_INTERNAL
    static EPFramebuffer *instance();
    #endif
    static EPFramebuffer *createControlledInstance();

    // Injected:
    QImage *getAuxFramebuffer();
    QImage *getMainFramebuffer();
};

EPFramebuffer *createEPFramebuffer();
