export const CONFIG_KEYS = [
    'inner-gap',
    'outer-gap',
    'split-ratio',
    'resize-step',
    'smart-split',
];

export function readConfig(settings) {
    return {
        inner_gap: settings.get_int('inner-gap'),
        outer_gap: settings.get_int('outer-gap'),
        split_ratio: settings.get_double('split-ratio'),
        resize_step: settings.get_double('resize-step'),
        smart_split: settings.get_boolean('smart-split'),
    };
}

export function ignoredApps(settings) {
    return new Set(settings.get_strv('ignored-apps').map(app => app.toLowerCase()));
}
