void h(void);

int f(int *p) {
    return *+p + *-p + *~p + *!h();
}
