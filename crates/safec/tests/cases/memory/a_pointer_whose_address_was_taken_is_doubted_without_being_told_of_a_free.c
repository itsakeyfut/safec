void *malloc(int n);
void free(void *p);
int *allocation(void) {
    int *p = malloc(4);
    int **pp = &p;
    return p;
}
int *parameter(int *p) {
    int **pp = &p;
    return p;
}
int freed_through_the_address(void) {
    int *p = malloc(4);
    if (!p) {
        return 0;
    }
    int **pp = &p;
    free(*pp);
    return *p;
}
