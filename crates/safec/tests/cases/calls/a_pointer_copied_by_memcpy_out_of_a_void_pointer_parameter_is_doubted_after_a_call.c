void *memcpy(void *d, void *s, int n);
void release(void);
int f(void *pp) {
    if (pp == 0) {
        return 0;
    }
    int *r = 0;
    memcpy(&r, pp, 8);
    int *s = r;
    if (s == 0) {
        return 0;
    }
    release();
    return *s;
}
