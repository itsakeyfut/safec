void *malloc(int n);
void keep(void *p);
void other(void);
void use1(int *p);
void run(void) {
    int x = 0;
    int *w = &x;
    int ***m = malloc(8);
    if (m == 0) {
        return;
    }
    *m = 0;
    keep(m);
    *m = &w;
    other();
    use1(w);
}
