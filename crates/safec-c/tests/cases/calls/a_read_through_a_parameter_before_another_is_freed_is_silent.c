void free(void *p);

int consume(int *p, int *q) {
    if (p == 0) {
        return 0;
    }
    if (q == 0) {
        return 0;
    }
    int x = *q;
    free(p);
    return x;
}
