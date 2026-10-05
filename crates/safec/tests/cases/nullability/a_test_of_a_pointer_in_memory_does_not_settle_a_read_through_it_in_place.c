int f(int **pp) {
    if (pp) {
        if (*pp) {
            return **pp;
        }
    }
    return 0;
}
