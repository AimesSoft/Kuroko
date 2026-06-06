#import <AppKit/AppKit.h>
#import <QuartzCore/CAMetalLayer.h>

void *erika_presenter_check_create_layer(double width, double height, double scale) {
    [NSApplication sharedApplication];
    CAMetalLayer *layer = [CAMetalLayer layer];
    layer.frame = CGRectMake(0, 0, width, height);
    layer.contentsScale = scale;
    layer.drawableSize = CGSizeMake(width * scale, height * scale);
    layer.pixelFormat = MTLPixelFormatBGRA8Unorm;
    return (__bridge_retained void *)layer;
}

void erika_presenter_check_release_layer(void *rawLayer) {
    if (rawLayer == NULL) {
        return;
    }
    CFRelease(rawLayer);
}
