void *malloc(int n);
void free(void *p);
void show(int *p);
void forget(void);

int f(int c) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    if (c) {
        show(p);
    }
    forget();
    return *p;
}
