void *malloc(unsigned long long size);
void free(void *pointer);

struct Buffer {
    int count;
    int items[];
};

int sized[sizeof(struct Buffer) == sizeof(int) ? 1 : -1];

int fits(void) {
    struct Buffer *b = malloc(sizeof(struct Buffer) + 4 * sizeof(int));
    if (!b) {
        return 1;
    }
    b->count = 4;
    b->items[3] = 7;
    int last = b->items[3];
    free(b);
    return last;
}

int past(void) {
    struct Buffer *b = malloc(sizeof(struct Buffer) + 4 * sizeof(int));
    if (!b) {
        return 1;
    }
    b->items[4] = 7;
    free(b);
    return 0;
}

int unknown(struct Buffer *_Nonnull b) {
    return b->items[3];
}

int on_the_stack(void) {
    struct Buffer b;
    b.count = 0;
    b.items[0] = 1;
    return b.items[0];
}

int main(void) {
    return fits() + past() + on_the_stack();
}
