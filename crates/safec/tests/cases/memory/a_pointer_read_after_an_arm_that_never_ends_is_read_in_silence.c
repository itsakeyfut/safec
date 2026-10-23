void *malloc(int n);
void free(void *p);
void work(void);
void use(int *p);

int f(int c, int *q) {
    int *p;
    if (c) {
        p = q;
    } else {
        for (;;) {
            work();
        }
    }
    use(p);
    return 0;
}
