struct Buffer {
    char tag;
    int items[];
};

union Holder {
    struct Buffer buffer;
    int other;
};

struct Buffer make(void) {
    struct Buffer b;
    b.tag = 1;
    return b;
}

int take(struct Buffer b) {
    return b.tag;
}

int held(union Holder h) {
    return h.other;
}
