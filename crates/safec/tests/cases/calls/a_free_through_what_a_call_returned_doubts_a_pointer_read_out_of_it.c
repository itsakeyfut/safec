void free(void *p);
int **get_slot(void);
int f(void) {
    int **pp = get_slot();
    if (pp == 0) {
        return 0;
    }
    int *r = *pp;
    if (r == 0) {
        return 0;
    }
    free(*pp);
    return *r;
}
