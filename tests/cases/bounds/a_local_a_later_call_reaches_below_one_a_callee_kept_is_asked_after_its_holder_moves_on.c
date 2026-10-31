void keep(void *p);
void other(void);
void other2(void);
void use1(int *p);
void run(void) {
    int z = 0;
    int *y = &z;
    int **w = 0;
    int ***v = 0;
    keep(&v);
    v = &w;
    other();
    v = 0;
    w = &y;
    other2();
    use1(y);
}
