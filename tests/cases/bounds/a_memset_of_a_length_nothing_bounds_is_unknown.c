void *malloc(int n);
void *memset(void *d, int c, int n);

int f(int n) {
    char *p = malloc(4);
    if (p == 0) {
        return 1;
    }
    memset(p, 0, n);
    return 0;
}
