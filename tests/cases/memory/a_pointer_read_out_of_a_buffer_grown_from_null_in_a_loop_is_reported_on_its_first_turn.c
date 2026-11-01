void *realloc(void *p, int n);
void free(void *p);
int f(int c) {
    int x = 1;
    int **buf = 0;
    int i = 0;
    while (i < c) {
        int **grown = realloc(buf, 8);
        if (!grown) { free(buf); return 1; }
        buf = grown;
        int *p = *buf;
        buf[0] = &x;
        i = i + 1;
        if (p == 0) return 2;
    }
    free(buf);
    return 0;
}
