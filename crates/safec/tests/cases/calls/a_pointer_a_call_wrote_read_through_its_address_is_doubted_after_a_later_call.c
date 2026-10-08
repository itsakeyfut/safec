void release(void);
void acquire(int **out);
int f(void) {
    int *r = 0;
    int **pr = &r;
    acquire(&r);
    int *s = *pr;
    if (s == 0) {
        return 0;
    }
    release();
    return *s;
}
