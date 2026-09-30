// Loaded into the real Compositor app with DYLD_INSERT_LIBRARIES by dump-main-menu.sh. Once the app has finished
// launching and SwiftUI has built its menu bar from CompositorApp's commands, this writes NSApp.mainMenu to standard
// output as JSON, between PARITY-MENUS-BEGIN and PARITY-MENUS-END lines, and exits the app.
//
// The fields match MenuDump.raw in parity/harness/Sources/MenuDump.swift, which normalizes both menus for
// menus.json. Each menu is updated first, as AppKit does before showing it, so enabled states are the ones it would
// show. The Services submenu is left empty: its contents depend on what else is installed.

#import <AppKit/AppKit.h>

static double settleSeconds(void) {
    const char *value = getenv("PARITY_MENUS_DELAY");
    double seconds = value ? atof(value) : 0;
    return seconds > 0 ? seconds : 3;
}

static NSArray *dumpMenu(NSMenu *menu) {
    id<NSMenuDelegate> delegate = menu.delegate;
    if ([delegate respondsToSelector:@selector(menuNeedsUpdate:)]) [delegate menuNeedsUpdate:menu];
    if ([delegate respondsToSelector:@selector(numberOfItemsInMenu:)] &&
        [delegate respondsToSelector:@selector(menu:updateItem:atIndex:shouldCancel:)]) {
        NSInteger count = [delegate numberOfItemsInMenu:menu];
        while (menu.numberOfItems < count) [menu addItem:[[NSMenuItem alloc] initWithTitle:@"" action:NULL keyEquivalent:@""]];
        while (menu.numberOfItems > count && count >= 0) [menu removeItemAtIndex:menu.numberOfItems - 1];
        for (NSInteger index = 0; index < count; index++) {
            if (![delegate menu:menu updateItem:[menu itemAtIndex:index] atIndex:index shouldCancel:NO]) break;
        }
    }
    [menu update];
    NSMutableArray *items = [NSMutableArray array];
    for (NSMenuItem *item in menu.itemArray) {
        if (item.isSeparatorItem) {
            [items addObject:@{@"separator": @YES}];
            continue;
        }
        NSMutableDictionary *entry = [@{
            @"title": item.title ?: @"",
            @"key": item.keyEquivalent ?: @"",
            @"modifiers": @(item.keyEquivalentModifierMask & NSEventModifierFlagDeviceIndependentFlagsMask),
            @"enabled": @(item.isEnabled),
            @"hidden": @(item.isHidden),
            @"state": @(item.state),
            @"alternate": @(item.isAlternate),
        } mutableCopy];
        if (item.submenu) {
            if (item.submenu == NSApp.servicesMenu) {
                entry[@"submenu"] = @[];
                entry[@"system"] = @"services";
            } else {
                entry[@"submenu"] = dumpMenu(item.submenu);
            }
        }
        [items addObject:entry];
    }
    return items;
}

static void dumpAndExit(NSTimeInterval launched) {
    NSMenu *main = NSApp.mainMenu;
    NSDictionary *capture = @{
        @"secondsAfterLaunch": @([NSDate timeIntervalSinceReferenceDate] - launched),
        @"appActive": @(NSApp.isActive),
        @"keyWindow": (id)NSApp.keyWindow.title ?: (id)[NSNull null],
        @"visibleWindows": @([NSApp.windows filteredArrayUsingPredicate:[NSPredicate predicateWithFormat:@"visible == YES"]].count),
        @"bundleIdentifier": (id)NSBundle.mainBundle.bundleIdentifier ?: (id)[NSNull null],
        @"version": (id)[NSBundle.mainBundle objectForInfoDictionaryKey:@"CFBundleShortVersionString"] ?: (id)[NSNull null],
    };
    NSDictionary *root = @{@"items": main ? dumpMenu(main) : @[], @"capture": capture};
    NSError *error = nil;
    NSData *data = [NSJSONSerialization dataWithJSONObject:root
                                                   options:NSJSONWritingPrettyPrinted | NSJSONWritingSortedKeys
                                                     error:&error];
    if (!data) {
        fprintf(stderr, "MenuDump: couldn't encode the menu: %s\n", error.localizedDescription.UTF8String);
        fflush(stderr);
        _exit(3);
    }
    fputs("\nPARITY-MENUS-BEGIN\n", stdout);
    fwrite(data.bytes, 1, data.length, stdout);
    fputs("\nPARITY-MENUS-END\n", stdout);
    fflush(stdout);
    // No termination handshake: the app would ask about unsaved work and wait on its delegate.
    _exit(0);
}

__attribute__((constructor)) static void parityMenuDumpInstall(void) {
    [[NSNotificationCenter defaultCenter] addObserverForName:NSApplicationDidFinishLaunchingNotification
                                                      object:nil
                                                       queue:[NSOperationQueue mainQueue]
                                                  usingBlock:^(NSNotification *note) {
        NSTimeInterval launched = [NSDate timeIntervalSinceReferenceDate];
        // Frontmost, as when a person uses the menu bar: some items follow the key window.
        [NSApp activateIgnoringOtherApps:YES];
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(settleSeconds() * NSEC_PER_SEC)), dispatch_get_main_queue(), ^{
            dumpAndExit(launched);
        });
    }];
}
