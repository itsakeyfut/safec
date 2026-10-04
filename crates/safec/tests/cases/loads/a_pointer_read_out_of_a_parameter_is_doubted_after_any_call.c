void log_line(void);

int peek(int **pp) {
    if (pp == 0) {
        return 0;
    }
    int *q = *pp;
    if (q == 0) {
        return 0;
    }
    log_line();
    return *q;
}
