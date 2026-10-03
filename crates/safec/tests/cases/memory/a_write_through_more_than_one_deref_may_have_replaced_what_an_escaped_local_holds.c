void *malloc(int n);
void free(void *p);
int f(int ***outer) {
    int *p = malloc(4);
    int *r = p;
    *outer = &p;
    int *q = malloc(8);
    **outer = q;
    free(p);
    *r = 1;
    return 0;
}
