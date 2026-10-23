void exit(int status);

int f(int *p) {
    *p += (exit(1), 0);
    return 0;
}
