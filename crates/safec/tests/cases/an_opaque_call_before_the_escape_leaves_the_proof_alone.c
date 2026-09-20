void *malloc(int n);
void free(void *p);
void other(void);
int f(void) {
    int *p = malloc(4);
    int *q = p;
    other();
    int **pp = &p;
    free(p);
    free(q);
    return 0;
}
