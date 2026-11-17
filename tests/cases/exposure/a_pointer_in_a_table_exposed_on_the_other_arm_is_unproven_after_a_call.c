void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
void **slot(void);
void forget(void);

int f(int c) {
    void **out_ = slot();
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    if (c) {
        memcpy(out_, &tab, 8);
    } else {
        *tab = p;
    }
    forget();
    return *p;
}
