int g(int *_Nonnull p);
int below(int **pp) {
    if (!pp) {
        return 0;
    }
    return **pp;
}
int passed(int **pp) {
    if (!pp) {
        return 0;
    }
    return g(*pp);
}
int *_Nonnull returned(int **pp) {
    if (!pp) {
        return 0;
    }
    return *pp;
}
