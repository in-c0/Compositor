// Traces the compute passes Core Image and Metal Performance Shaders encode, by swizzling the
// command buffer's compute encoder factories. Run on a Mac:
//
//   clang -fobjc-arc -framework Foundation -framework Metal -framework MetalPerformanceShaders \
//       -framework CoreImage -framework CoreGraphics trace_passes.m -o trace_passes
//   ./trace_passes ci                        # Core Image's passes for a few Gaussian sigmas
//   ./trace_passes mps 20 5000 > table.txt   # MPSImageGaussianBlur's weights, sigma = k / 20
//
// `pack.py table.txt` turns the second into engine/src/adjust/mps_gaussian.bin.
#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <MetalPerformanceShaders/MetalPerformanceShaders.h>
#import <CoreImage/CoreImage.h>
#import <objc/runtime.h>

static BOOL tracing = NO;
static BOOL compact = NO;

@interface LogProxy : NSProxy
@property (strong) id target;
@end

@implementation LogProxy
- (NSMethodSignature *)methodSignatureForSelector:(SEL)sel { return [self.target methodSignatureForSelector:sel]; }
- (void)forwardInvocation:(NSInvocation *)inv {
    NSString *name = NSStringFromSelector(inv.selector);
    if ([name hasPrefix:@"setComputePipelineState"]) {
        __unsafe_unretained id state; [inv getArgument:&state atIndex:2];
        NSString *desc = [state description]; NSRange r = [desc rangeOfString:@"label = "];
        NSString *label = r.location != NSNotFound ? [[desc substringFromIndex:r.location + r.length] componentsSeparatedByString:@"\n"][0] : @"?";
        printf(compact ? " | %s" : "  P %s\n", label.UTF8String);
    } else if ([name isEqualToString:@"setBytes:length:atIndex:"]) {
        const float *p; NSUInteger n, idx; [inv getArgument:&p atIndex:2]; [inv getArgument:&n atIndex:3]; [inv getArgument:&idx atIndex:4];
        if (compact) {
            for (NSUInteger k = 12; k < n / 4 && k < 28; k++) printf(" %.9g", p[k]);
        } else {
            printf("    bytes %lu @%lu:", (unsigned long)n, (unsigned long)idx);
            const int32_t *q = (const int32_t *)p;
            for (NSUInteger k = 0; k < n / 4 && k < 48; k++) printf(" %.9g|%d", p[k], q[k]);
            printf("\n");
        }
    } else if (!compact && [name isEqualToString:@"setTexture:atIndex:"]) {
        __unsafe_unretained id<MTLTexture> t; NSUInteger idx; [inv getArgument:&t atIndex:2]; [inv getArgument:&idx atIndex:3];
        if (t) printf("    tex %lu %lux%lu fmt %lu\n", (unsigned long)idx, (unsigned long)t.width, (unsigned long)t.height, (unsigned long)t.pixelFormat);
    } else if (!compact && [name hasPrefix:@"setBuffer:offset:atIndex:"]) {
        __unsafe_unretained id<MTLBuffer> b; NSUInteger off, idx; [inv getArgument:&b atIndex:2]; [inv getArgument:&off atIndex:3]; [inv getArgument:&idx atIndex:4];
        printf("    buffer @%lu len %lu\n", (unsigned long)idx, (unsigned long)b.length);
        if (b && b.storageMode != MTLStorageModePrivate) {
            const float *p = (const float *)((const char *)b.contents + off);
            printf("      "); for (NSUInteger k = 0; k < MIN((b.length - off) / 4, (NSUInteger)48); k++) printf(" %.9g", p[k]); printf("\n");
        }
    } else if (!compact && [name hasPrefix:@"dispatch"]) {
        MTLSize a, b; [inv getArgument:&a atIndex:2]; [inv getArgument:&b atIndex:3];
        printf("    dispatch (%lu %lu) (%lu %lu)\n", (unsigned long)a.width, (unsigned long)a.height, (unsigned long)b.width, (unsigned long)b.height);
    }
    [inv invokeWithTarget:self.target];
}
@end

static IMP origEnc, origEncType, origEncDesc;
static id wrap(id enc) {
    if (!tracing || !enc) return enc;
    LogProxy *p = [LogProxy alloc]; p.target = enc; CFBridgingRetain(p); return p;
}
static id hookEnc(id self, SEL _cmd) { return wrap(((id (*)(id, SEL))origEnc)(self, _cmd)); }
static id hookEncType(id self, SEL _cmd, NSUInteger t) { return wrap(((id (*)(id, SEL, NSUInteger))origEncType)(self, _cmd, t)); }
static id hookEncDesc(id self, SEL _cmd, id d) { return wrap(((id (*)(id, SEL, id))origEncDesc)(self, _cmd, d)); }

static void swizzle(Class c, SEL s, IMP hook, IMP *orig) {
    Method m = class_getInstanceMethod(c, s);
    if (m) *orig = method_setImplementation(m, hook);
}

int main(int argc, const char **argv) {
    @autoreleasepool {
        id<MTLDevice> device = MTLCreateSystemDefaultDevice();
        id<MTLCommandQueue> queue = [device newCommandQueue];
        Class cbc = object_getClass([queue commandBuffer]);
        swizzle(cbc, @selector(computeCommandEncoder), (IMP)hookEnc, &origEnc);
        swizzle(cbc, @selector(computeCommandEncoderWithDispatchType:), (IMP)hookEncType, &origEncType);
        swizzle(cbc, @selector(computeCommandEncoderWithDescriptor:), (IMP)hookEncDesc, &origEncDesc);
        NSString *mode = argc > 1 ? @(argv[1]) : @"ci";
        if ([mode isEqualToString:@"ci"]) {
            // Core Image's own passes for a Gaussian blur of a 64x64 8-bit image.
            CIContext *ctx = [CIContext contextWithOptions:@{kCIContextWorkingColorSpace: [NSNull null], kCIContextOutputColorSpace: [NSNull null]}];
            CGColorSpaceRef srgb = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
            CGContextRef c = CGBitmapContextCreate(NULL, 64, 64, 8, 256, srgb, kCGImageAlphaPremultipliedLast | kCGBitmapByteOrder32Big);
            CGContextSetRGBFillColor(c, 1, 0.5, 0.25, 1); CGContextFillRect(c, CGRectMake(16, 16, 32, 32));
            CIImage *img = [CIImage imageWithCGImage:CGBitmapContextCreateImage(c)];
            double sigmas[] = {0.3, 0.5, 0.75, 1.0, 1.1, 1.2, 1.25, 3.0};
            for (int i = 0; i < 8; i++) {
                printf("SIGMA %g\n", sigmas[i]);
                tracing = YES;
                CIImage *b = [[img imageByApplyingGaussianBlurWithSigma:sigmas[i]] imageByCroppingToRect:CGRectMake(0, 0, 64, 64)];
                CGImageRef out = [ctx createCGImage:b fromRect:CGRectMake(0, 0, 64, 64) format:kCIFormatRGBA8 colorSpace:srgb];
                CGImageRelease(out);
                tracing = NO;
                fflush(stdout);
            }
        } else {
            // MPS's parameters: sigma = k / 20 for k in [lo, hi].
            compact = YES;
            int lo = atoi(argv[2]), hi = atoi(argv[3]);
            MTLTextureDescriptor *d = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatRGBA16Float width:64 height:64 mipmapped:NO];
            d.usage = MTLTextureUsageShaderRead | MTLTextureUsageShaderWrite;
            id<MTLTexture> src = [device newTextureWithDescriptor:d], dst = [device newTextureWithDescriptor:d];
            for (int k = lo; k <= hi; k++) {
                float sigma = (float)((double)k / 20.0);
                printf("%d %.9g", k, sigma);
                MPSImageGaussianBlur *blur = [[MPSImageGaussianBlur alloc] initWithDevice:device sigma:sigma];
                id<MTLCommandBuffer> cb = [queue commandBuffer];
                tracing = YES;
                [blur encodeToCommandBuffer:cb sourceTexture:src destinationTexture:dst];
                tracing = NO;
                [cb commit]; [cb waitUntilCompleted];
                printf("\n");
            }
        }
    }
    return 0;
}
