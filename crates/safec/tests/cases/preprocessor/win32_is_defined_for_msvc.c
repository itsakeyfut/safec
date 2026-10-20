#ifdef _WIN32
int win;
#else
int other;
#endif
#ifdef __linux__
int linux_only;
#endif
#ifdef __STDC__
int stdc;
#endif

int main(void) { return 0; }
