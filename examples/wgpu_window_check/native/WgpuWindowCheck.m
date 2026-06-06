#import <AppKit/AppKit.h>
#import <QuartzCore/CAMetalLayer.h>

// Creates a visible titled window hosting a CAMetalLayer and returns the layer.
// The wgpu renderer attaches to this layer as a CoreAnimationLayer surface.
void *erika_wgpu_window_create(double width, double height, double scale) {
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];

    NSRect frame = NSMakeRect(120, 120, width, height);
    NSWindow *window = [[NSWindow alloc]
        initWithContentRect:frame
                  styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable)
                    backing:NSBackingStoreBuffered
                      defer:NO];
    [window setTitle:@"Erika wgpu"];
    [window setReleasedWhenClosed:NO];

    NSView *view = [window contentView];
    [view setWantsLayer:YES];

    CAMetalLayer *layer = [CAMetalLayer layer];
    layer.frame = view.bounds;
    layer.contentsScale = scale;
    layer.drawableSize = CGSizeMake(width * scale, height * scale);
    layer.pixelFormat = MTLPixelFormatBGRA8Unorm;
    [view setLayer:layer];

    [window center];
    [window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];

    // Pump events briefly so the window actually appears on screen.
    for (int i = 0; i < 40; i++) {
        NSEvent *event;
        while ((event = [NSApp nextEventMatchingMask:NSEventMaskAny
                                           untilDate:[NSDate dateWithTimeIntervalSinceNow:0.005]
                                              inMode:NSDefaultRunLoopMode
                                             dequeue:YES])) {
            [NSApp sendEvent:event];
        }
    }
    return (__bridge_retained void *)layer;
}

// Drains pending AppKit events so the window stays responsive between frames.
void erika_wgpu_window_pump(void) {
    NSEvent *event;
    while ((event = [NSApp nextEventMatchingMask:NSEventMaskAny
                                       untilDate:[NSDate dateWithTimeIntervalSinceNow:0.001]
                                          inMode:NSDefaultRunLoopMode
                                         dequeue:YES])) {
        [NSApp sendEvent:event];
    }
}

void erika_wgpu_window_release(void *rawLayer) {
    if (rawLayer != NULL) {
        CFRelease(rawLayer);
    }
}
