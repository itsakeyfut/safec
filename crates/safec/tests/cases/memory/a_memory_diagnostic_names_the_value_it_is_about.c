void free(void *p);
void g(int *p);
void twice(int *p) {
    free(p);
    free(p);
}
int through_another(int *p) {
    int *q = p;
    free(q);
    return *p;
}
int both(int *p, int *q) {
    free(p);
    free(q);
    return *p + *q;
}
int first_level(int **pp) {
    free(pp);
    return **pp;
}
int *returned(int *p) {
    free(p);
    return p;
}
void passed(int *p) {
    free(p);
    g(p);
}
void lost(int **pp) {
    int *p = *pp;
    free(p);
}
void interior(int *p) {
    int *q = p + 1;
    free(q);
}
