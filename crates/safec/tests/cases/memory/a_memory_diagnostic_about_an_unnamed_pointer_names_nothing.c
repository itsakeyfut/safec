void *malloc(int n);
void free(void *p);
int subscript(int *p) {
    free(p);
    return p[2];
}
void through_memory(int **pp) {
    free(*pp);
}
int deeper(void) {
    int **pp = malloc(8);
    int *p = malloc(4);
    if (!pp || !p) {
        return 0;
    }
    *pp = p;
    free(p);
    return **pp;
}
