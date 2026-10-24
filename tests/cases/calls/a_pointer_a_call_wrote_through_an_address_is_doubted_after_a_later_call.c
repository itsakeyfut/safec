void release(void);
void acquire(int **out);
int f(void) {
    int *r = 0;
    acquire(&r);
    int *s = r;
    if (s == 0) {
        return 0;
    }
    release();
    return *s;
}
