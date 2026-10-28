void *malloc(int n);
void use3(int ***h);
void log_it(void);
int f(int *p) {
    int ***h = malloc(8);
    if (h == 0) { return 1; }
    int **b = malloc(8);
    if (b == 0) { return 1; }
    *b = p;
    *h = b;
    log_it();
    use3(h);
    return 0;
}
