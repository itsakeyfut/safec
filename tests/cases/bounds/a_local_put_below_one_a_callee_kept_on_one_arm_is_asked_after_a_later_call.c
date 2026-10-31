void keep(void *p);
void other(void);
void use1(int *p);
void run(int c) {
    int x = 0;
    int *w = &x;
    int **v = 0;
    if (c) {
        keep(&v);
    }
    v = &w;
    other();
    use1(w);
}
