void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    int **pp = &p;
    free(p);
    *pp = malloc(8);
    free(p);
    return 0;
}
