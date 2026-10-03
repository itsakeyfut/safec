void *malloc(int n);
void free(void *p);
void *memset(void *s, int c, int n);
void forget(void);

int f(int c) {
    int **tab = malloc(8);
    if (tab == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    if (c) {
        memset(tab, 0, 8);
    } else {
        *tab = p;
    }
    forget();
    return *p;
}
