void *memcpy(void *d, void *s, int n);
void release(void);
void acquire(int **out);
int f(void) {
    int *r = 0;
    acquire(&r);
    int *s = 0;
    memcpy(&s, &r, 8);
    int *t = s;
    if (t == 0) {
        return 0;
    }
    release();
    return *t;
}
