void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void **slot(void);
int release_all(void);
int use(void *p);
int f(void) {
    void **out_ = slot();
    int r;
    int *a;
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    r = release_all() + use(memcpy(out_, &a, 8));
    return r;
}
