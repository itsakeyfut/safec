int f(int **pp) {
    if (pp) {
        return **pp;
    }
    return 0;
}
