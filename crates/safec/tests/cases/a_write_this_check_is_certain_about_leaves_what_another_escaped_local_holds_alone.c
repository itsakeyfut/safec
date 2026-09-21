void *malloc(int n);
void free(void *p);
int f(int ***outer) {
    int *o = malloc(4);
    int *ro = o;
    *outer = &o;
    int *p = malloc(8);
    int **pp = &p;
    *pp = 0;
    free(o);
    free(ro);
    return 0;
}
