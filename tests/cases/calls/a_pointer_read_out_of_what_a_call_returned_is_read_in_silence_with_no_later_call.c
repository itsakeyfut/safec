int **get_slot(void);
int f(void) {
    int **pp = get_slot();
    if (pp == 0) {
        return 0;
    }
    int *q = *pp;
    if (q == 0) {
        return 0;
    }
    return *q;
}
