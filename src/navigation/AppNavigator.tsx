import React, { useState, useEffect } from 'react';
import { NavigationContainer } from '@react-navigation/native';
import { createBottomTabNavigator } from '@react-navigation/bottom-tabs';
import { Ionicons } from '@expo/vector-icons';
import Constants from 'expo-constants';
import * as SecureStore from 'expo-secure-store';
import { useTheme } from '../styles/theme';

import LibraryScreen from '../screens/LibraryScreen';
import SimpleAccountScreen from '../screens/SimpleAccountScreen';
import SettingsScreen from '../screens/SettingsScreen';
import TaskDebugScreen from '../screens/TaskDebugScreen';

const Tab = createBottomTabNavigator();
const DEBUG_MODE_KEY = 'debug_mode_enabled';

export default function AppNavigator() {
  const { colors } = useTheme();
  const [enableDebugScreen, setEnableDebugScreen] = useState<boolean>(
    Constants.expoConfig?.extra?.enableDebugScreen ?? __DEV__
  );

  // Check if debug mode is enabled/disabled via SecureStore (secret activation/deactivation)
  useEffect(() => {
    const checkDebugMode = async () => {
      try {
        const debugEnabled = await SecureStore.getItemAsync(DEBUG_MODE_KEY);
        // If user has explicitly set debug mode via secret gesture, respect that setting
        if (debugEnabled === 'true') {
          setEnableDebugScreen(true);
        } else if (debugEnabled === 'false') {
          setEnableDebugScreen(false);
        }
        // Otherwise, fall back to env var / __DEV__ (initial state)
      } catch (error) {
        console.error('[AppNavigator] Failed to check debug mode:', error);
      }
    };
    checkDebugMode();
  }, []);

  return (
    <NavigationContainer>
      <Tab.Navigator
        screenOptions={{
          tabBarStyle: {
            backgroundColor: colors.backgroundSecondary,
            borderTopColor: colors.border,
            borderTopWidth: 1,
          },
          tabBarActiveTintColor: colors.accent,
          tabBarInactiveTintColor: colors.textSecondary,
          headerStyle: {
            backgroundColor: colors.backgroundSecondary,
            borderBottomColor: colors.border,
            borderBottomWidth: 1,
          },
          headerTintColor: colors.textPrimary,
          headerTitleStyle: {
            fontWeight: '600',
          },
        }}
      >
        <Tab.Screen
          name="Library"
          component={LibraryScreen}
          options={{
            tabBarLabel: 'Library',
            headerShown: false,
            tabBarIcon: ({ color, size }) => (
              <Ionicons name="library" size={size} color={color} />
            ),
          }}
        />
        <Tab.Screen
          name="Account"
          component={SimpleAccountScreen}
          options={{
            tabBarLabel: 'Account',
            headerShown: false,
            tabBarIcon: ({ color, size }) => (
              <Ionicons name="person" size={size} color={color} />
            ),
          }}
        />
        <Tab.Screen
          name="Settings"
          component={SettingsScreen}
          options={{
            tabBarLabel: 'Settings',
            headerShown: false,
            tabBarIcon: ({ color, size }) => (
              <Ionicons name="settings" size={size} color={color} />
            ),
          }}
        />
        {enableDebugScreen && (
          <Tab.Screen
            name="Debug"
            component={TaskDebugScreen}
            options={{
              tabBarLabel: 'Debug',
              headerShown: false,
              tabBarIcon: ({ color, size }) => (
                <Ionicons name="bug" size={size} color={color} />
              ),
            }}
          />
        )}
      </Tab.Navigator>
    </NavigationContainer>
  );
}
