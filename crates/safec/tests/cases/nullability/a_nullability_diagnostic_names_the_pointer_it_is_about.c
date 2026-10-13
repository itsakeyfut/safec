int g(int *_Nonnull p);
int proved(void) {
    int *p = 0;
    return *p;
}
int unproven(int *p) {
    return *p;
}
int written_through(int *q) {
    int *p = 0;
    *q = *p;
    return 0;
}
int beside(int *p, int *q) {
    return *p + *q;
}
int first_level(int **pp) {
    return **pp;
}
int passed_null(void) {
    int *p = 0;
    return g(p);
}
int passed_unproven(int *p) {
    return g(p);
}
int *_Nonnull returned_null(void) {
    int *p = 0;
    return p;
}
int *_Nonnull returned_unproven(int *p) {
    return p;
}
