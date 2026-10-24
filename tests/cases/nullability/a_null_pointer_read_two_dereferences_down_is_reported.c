int f(void) {
    int *p = 0;
    int **pp = &p;
    return **pp;
}

int g(void) {
    int *p = 0;
    int **pp = &p;
    return **pp + 1;
}
