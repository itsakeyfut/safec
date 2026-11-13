// A freed pointer two levels below what leaves the function: the store and
// the library copy of the same `b` are one write, and each is `SC0414` at
// the write (the memory model rebuilt on #924, stage 2). Asked by the store
// alone, the copy was the argument door's `SC0407`.
void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int stored(int ****out) {
    int *x = malloc(4); if (x == 0) return 0;
    int **box = malloc(8); if (box == 0) return 0;
    int ***b = malloc(8); if (b == 0) return 0;
    *box = x; *b = box; free(x);
    if (out == 0) return 0;
    *out = b;
    return 1;
}
int copied(int ****out) {
    int *x = malloc(4); if (x == 0) return 0;
    int **box = malloc(8); if (box == 0) return 0;
    int ***b = malloc(8); if (b == 0) return 0;
    *box = x; *b = box; free(x);
    if (out == 0) return 0;
    memcpy(out, &b, 8);
    return 1;
}
int stored_maybe(int ***out, int c) {
    int *x = malloc(4); if (x == 0) return 0;
    int **b = malloc(8); if (b == 0) return 0;
    *b = x; if (c) free(x);
    if (out == 0) return 0;
    *out = b;
    return 1;
}
int copied_maybe(int ***out, int c) {
    int *x = malloc(4); if (x == 0) return 0;
    int **b = malloc(8); if (b == 0) return 0;
    *b = x; if (c) free(x);
    if (out == 0) return 0;
    memcpy(out, &b, 8);
    return 1;
}
