void exit_now(void) {
}

int f(int *_Nullable p) {
    exit_now();
    return *p;
}
