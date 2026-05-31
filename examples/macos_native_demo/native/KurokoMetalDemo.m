#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>

extern void kuroko_demo_attach_layer(void *layer, unsigned int width, unsigned int height, double scale);
extern void kuroko_demo_resize_layer(unsigned int width, unsigned int height, double scale);
extern void kuroko_demo_render_frame(double time_seconds);
extern double kuroko_demo_smoke_seconds(void);

@interface KurokoMetalDemoView : NSView
@property(nonatomic, strong) CAMetalLayer *metalLayer;
@property(nonatomic, strong) NSTimer *timer;
@property(nonatomic, assign) CFTimeInterval startTime;
@end

@implementation KurokoMetalDemoView

- (instancetype)initWithFrame:(NSRect)frameRect {
  self = [super initWithFrame:frameRect];
  if (self) {
    self.wantsLayer = YES;
    self.metalLayer = [CAMetalLayer layer];
    self.metalLayer.pixelFormat = MTLPixelFormatBGRA8Unorm;
    self.metalLayer.framebufferOnly = YES;
    self.metalLayer.opaque = YES;
    self.layer = self.metalLayer;
    self.startTime = CACurrentMediaTime();
  }
  return self;
}

- (BOOL)wantsUpdateLayer {
  return YES;
}

- (void)viewDidMoveToWindow {
  [super viewDidMoveToWindow];
  [self updateDrawableSizeAndAttach:YES];
  if (self.window != nil && self.timer == nil) {
    self.timer = [NSTimer scheduledTimerWithTimeInterval:(1.0 / 60.0)
                                                  target:self
                                                selector:@selector(renderTick:)
                                                userInfo:nil
                                                 repeats:YES];
    [[NSRunLoop mainRunLoop] addTimer:self.timer forMode:NSRunLoopCommonModes];
  }
}

- (void)viewWillMoveToWindow:(NSWindow *)newWindow {
  if (newWindow == nil) {
    [self.timer invalidate];
    self.timer = nil;
  }
  [super viewWillMoveToWindow:newWindow];
}

- (void)setFrameSize:(NSSize)newSize {
  [super setFrameSize:newSize];
  [self updateDrawableSizeAndAttach:NO];
}

- (void)viewDidChangeBackingProperties {
  [super viewDidChangeBackingProperties];
  [self updateDrawableSizeAndAttach:NO];
}

- (void)updateDrawableSizeAndAttach:(BOOL)attach {
  CGFloat scale = self.window.backingScaleFactor > 0 ? self.window.backingScaleFactor : NSScreen.mainScreen.backingScaleFactor;
  CGSize drawableSize = CGSizeMake(MAX(1.0, self.bounds.size.width * scale), MAX(1.0, self.bounds.size.height * scale));
  self.metalLayer.drawableSize = drawableSize;
  self.metalLayer.frame = self.bounds;
  if (attach) {
    kuroko_demo_attach_layer((__bridge void *)self.metalLayer, (unsigned int)self.bounds.size.width, (unsigned int)self.bounds.size.height, scale);
  } else {
    kuroko_demo_resize_layer((unsigned int)self.bounds.size.width, (unsigned int)self.bounds.size.height, scale);
  }
}

- (void)renderTick:(NSTimer *)timer {
  (void)timer;
  double elapsed = CACurrentMediaTime() - self.startTime;
  kuroko_demo_render_frame(elapsed);
}

@end

@interface KurokoMetalDemoDelegate : NSObject <NSApplicationDelegate>
@property(nonatomic, strong) NSWindow *window;
@property(nonatomic, strong) NSTimer *smokeTimer;
@end

@implementation KurokoMetalDemoDelegate

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
  (void)notification;
  NSRect frame = NSMakeRect(0, 0, 960, 540);
  self.window = [[NSWindow alloc] initWithContentRect:frame
                                            styleMask:(NSWindowStyleMaskTitled |
                                                       NSWindowStyleMaskClosable |
                                                       NSWindowStyleMaskMiniaturizable |
                                                       NSWindowStyleMaskResizable)
                                              backing:NSBackingStoreBuffered
                                                defer:NO];
  self.window.title = @"Kuroko Metal Demo";
  self.window.contentView = [[KurokoMetalDemoView alloc] initWithFrame:frame];
  [self.window center];
  [self.window makeKeyAndOrderFront:nil];
  double smokeSeconds = kuroko_demo_smoke_seconds();
  if (smokeSeconds > 0.0) {
    self.smokeTimer = [NSTimer scheduledTimerWithTimeInterval:smokeSeconds
                                                       target:self
                                                     selector:@selector(smokeTimerFired:)
                                                     userInfo:nil
                                                      repeats:NO];
    [[NSRunLoop mainRunLoop] addTimer:self.smokeTimer forMode:NSRunLoopCommonModes];
  } else {
    [NSApp activateIgnoringOtherApps:YES];
  }
}

- (void)smokeTimerFired:(NSTimer *)timer {
  (void)timer;
  [NSApp terminate:nil];
}

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender {
  (void)sender;
  return YES;
}

@end

void kuroko_demo_run_app(void) {
  @autoreleasepool {
    NSApplication *app = [NSApplication sharedApplication];
    app.activationPolicy = NSApplicationActivationPolicyRegular;
    KurokoMetalDemoDelegate *delegate = [[KurokoMetalDemoDelegate alloc] init];
    app.delegate = delegate;
    [app run];
  }
}
