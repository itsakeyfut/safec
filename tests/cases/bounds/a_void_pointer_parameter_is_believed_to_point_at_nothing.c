void *memset(void *d, int c, int n);

void clear(void *p) {
    if (p != 0) {
        memset(p, 0, 1);
    }
}
