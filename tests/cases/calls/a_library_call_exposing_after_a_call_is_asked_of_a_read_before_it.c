void *malloc(int n);
void *memcpy(void *d, void *s, int n);
void **slot(void);
int release_all(void);
void stash(int **pp);
int f(void) {
    void **out_ = slot();
    int x;
    int r;
    int *a;
    int *holder;
    holder = 0;
    stash(&holder);
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    r = release_all() + (x = a[0]) + (memcpy(out_, &a, 8) != 0);
    return r;
}
