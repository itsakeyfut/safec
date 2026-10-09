int plain_first(int c, int *p, int **q) {
    return c ? *p : **q;
}

int through_memory_first(int c, int *p, int **q) {
    return c ? **q : *p;
}
