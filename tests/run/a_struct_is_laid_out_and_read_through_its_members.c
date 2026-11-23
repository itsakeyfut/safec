void *malloc(unsigned long long size);
void free(void *pointer);

struct Pair {
    char tag;
    long long wide;
    short narrow;
    int *pointer;
};

struct Counter {
    int counts[3];
    int total;
};

struct Pair global;

static int add(struct Counter *c) {
    c->total = c->counts[0] + c->counts[1] + c->counts[2];
    return c->total;
}

int main(void) {
    int r = 0;
    int x = 7;
    struct Pair local;
    local.tag = 1;
    local.wide = 2;
    local.narrow = 3;
    local.pointer = &x;
    r = r + (local.tag + local.wide + local.narrow + *local.pointer == 13 ? 0 : 1);

    struct Pair *p = &local;
    p->narrow = 30;
    r = r + (local.narrow == 30 ? 0 : 1);
    int *member = &p->narrow == &local.narrow ? &x : 0;
    r = r + (member == &x ? 0 : 1);

    global.wide = 40;
    r = r + (global.wide + global.tag == 40 ? 0 : 1);

    struct Counter counter;
    counter.counts[0] = 1;
    counter.counts[1] = 2;
    counter.counts[2] = 3;
    r = r + (add(&counter) == 6 && counter.total == 6 ? 0 : 1);

    struct Pair *heap = malloc(sizeof *heap);
    if (!heap) {
        return 1;
    }
    heap->pointer = &x;
    heap->wide = 5;
    r = r + (*heap->pointer + heap->wide == 12 ? 0 : 1);
    free(heap);

    r = r + (sizeof(struct Pair) == 32 || sizeof(struct Pair) == 24 ? 0 : 1);
    return r;
}
