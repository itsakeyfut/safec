void *malloc(int n);
void free(void *p);
int f(void) {
    int *a = malloc(4);
    int *p = malloc(4);
    int **pp = &p;
    free(a);
    *pp = a;
    free(a);
    return 0;
}
