void *malloc(int n);
void *memset(void *s, int c, int n);
int release_all(void);
int use(void *p);
int f(void) {
    int r;
    int *a;
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    r = release_all() + use(memset(a, 0, 4));
    return r;
}
