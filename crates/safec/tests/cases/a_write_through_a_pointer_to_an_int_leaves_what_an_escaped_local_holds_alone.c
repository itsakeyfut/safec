void *malloc(int n);
void free(void *p);
int f(int ***outer) {
    int *p = malloc(4);
    int *r = p;
    *outer = &p;
    *r = 1;
    free(p);
    free(r);
    return 0;
}
